// eapm -- EAP methods as state machines without I/O: the peer and the server of EAP-MD5 and
// EAP-PWD with the framing of netpkt::eap and the sums of crypto (RFC 3748, 5931).  They take the
// packets they get and give the packets to send; whoever holds them carries the bytes (EAPOL,
// RADIUS), keeps the timers and repeats a Request that got no answer.  Randomness comes from
// the caller, so a run is the same byte for byte
//
//   Method                         Md5 (wired ports, no keys) | Pwd (makes an MSK); word, parse,
//                                  of_type, typ, makes_keys
//   Peer::new(identity, password, method)
//   Peer::on_packet(&b, rnd) -> PeerOut   Send(bytes) | Success | Failure | Silent
//   Peer::msk() / emsk() / method_used()  after Success
//   Server::new(server_id, methods)       methods in the order to offer them
//   Server::on_packet(&b, lookup, rnd) -> ServerOut
//                                  Send(bytes) | Success { eap, msk, emsk } | Failure { eap } |
//                                  Silent; lookup: a user's password (None: no such user)
//   Server::user() / method()      who is logging in, with which method now
//   rand_scalar(rnd)               a scalar of P-256 from random bytes
//
// The peer answers a Request again with the same bytes when its identifier repeats (RFC 3748
// 4.1) and takes a method it was not told to use with a Nak that names the one it knows.  A
// Success before the method is over is ignored, for EAP-PWD unless its Confirm checked out
// (RFC 3748 4.2 and 7.2); a Confirm that does not fit (another password) ends the method without
// an answer, as RFC 5931 2.8.5.3 says, so the authenticator's retries run out.
//
// The server starts when the first Response/Identity comes.  It offers its methods in order and
// goes on to the one a Nak names if it has it; with none left, it answers with Failure.
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;

use crypto::eappwd;
use netpkt::eap::{self, Md5Data, Packet, Pwd, PwdId};

/// where random bytes come from
pub type Rnd<'a> = &'a mut dyn FnMut(&mut [u8]);

/// an EAP method
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    Md5,
    Pwd,
}

impl Method {
    /// its word in a wpa_supplicant-style file ("MD5", "PWD")
    pub fn word(self) -> &'static str {
        match self {
            Method::Md5 => "MD5",
            Method::Pwd => "PWD",
        }
    }
    /// by its word, in any case
    pub fn parse(s: &str) -> Option<Method> {
        match s.trim().to_ascii_uppercase().as_str() {
            "MD5" | "EAP-MD5" => Some(Method::Md5),
            "PWD" | "EAP-PWD" => Some(Method::Pwd),
            _ => None,
        }
    }
    /// the EAP type
    pub fn typ(self) -> u8 {
        match self {
            Method::Md5 => eap::T_MD5,
            Method::Pwd => eap::T_PWD,
        }
    }
    pub fn of_type(t: u8) -> Option<Method> {
        match t {
            eap::T_MD5 => Some(Method::Md5),
            eap::T_PWD => Some(Method::Pwd),
            _ => None,
        }
    }
    /// it derives an MSK (WPA-Enterprise needs one)
    pub fn makes_keys(self) -> bool {
        self == Method::Pwd
    }
}

/// a scalar in [1, n - 1] of P-256 from random bytes
pub fn rand_scalar(rnd: Rnd) -> [u8; 32] {
    loop {
        let mut k = [0u8; 32];
        rnd(&mut k);
        if crypto::p256::scalar_ok(&k) {
            return k;
        }
    }
}

/// one side's commit: both random scalars and what they give
fn draw_commit(pwe: &[u8; 64], rnd: Rnd) -> ([u8; 32], [u8; 32], [u8; 64]) {
    loop {
        let rand = rand_scalar(rnd);
        let mask = rand_scalar(rnd);
        if let Some((scalar, element)) = eappwd::commit(pwe, &rand, &mask) {
            return (rand, scalar, element);
        }
    }
}

/// what a peer does with a packet
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PeerOut {
    /// send this packet
    Send(Vec<u8>),
    /// EAP-Success after the method is over: msk() holds its keys
    Success,
    /// EAP-Failure, or a method that ended here
    Failure,
    /// nothing to do
    Silent,
}

/// where the peer's EAP-PWD is
#[derive(Clone, Debug)]
enum PwdPeer {
    Idle,
    Commit { token: [u8; 4], server: Vec<u8> },
    Confirm { ss: [u8; 32], elem: [u8; 64], scal: [u8; 32], s_elem: [u8; 64], s_scal: [u8; 32] },
    Done,
}

/// the peer's side of EAP
#[derive(Clone, Debug)]
pub struct Peer {
    pub identity: String,
    password: String,
    method: Method,
    pwd: PwdPeer,
    /// a method ran to its end here (EAP-MD5 answered, EAP-PWD confirmed)
    finished: bool,
    last: Option<(u8, Vec<u8>)>,
    msk: Option<[u8; 64]>,
    emsk: Option<[u8; 64]>,
    used: Option<Method>,
}

impl Peer {
    pub fn new(identity: &str, password: &str, method: Method) -> Peer {
        Peer { identity: String::from(identity), password: String::from(password), method, pwd: PwdPeer::Idle, finished: false, last: None, msk: None, emsk: None, used: None }
    }
    pub fn msk(&self) -> Option<&[u8; 64]> {
        self.msk.as_ref()
    }
    pub fn emsk(&self) -> Option<&[u8; 64]> {
        self.emsk.as_ref()
    }
    /// the method that ran
    pub fn method_used(&self) -> Option<Method> {
        self.used
    }
    /// back to the start (a new authentication on the same port)
    pub fn reset(&mut self) {
        self.pwd = PwdPeer::Idle;
        self.finished = false;
        self.last = None;
        self.msk = None;
        self.emsk = None;
        self.used = None;
    }

    fn answer(&mut self, id: u8, p: Packet) -> PeerOut {
        let b = p.to_bytes();
        self.last = Some((id, b.clone()));
        PeerOut::Send(b)
    }

    /// a packet from the authenticator
    pub fn on_packet(&mut self, b: &[u8], rnd: Rnd) -> PeerOut {
        let Some(p) = Packet::parse(b) else {
            return PeerOut::Silent;
        };
        match p.code {
            eap::SUCCESS => {
                if self.finished { PeerOut::Success } else { PeerOut::Silent }
            }
            eap::FAILURE => PeerOut::Failure,
            eap::REQUEST => self.request(&p, rnd),
            _ => PeerOut::Silent,
        }
    }

    fn request(&mut self, p: &Packet, rnd: Rnd) -> PeerOut {
        // a Request again: the same answer, nothing computed again
        if let Some((id, last)) = &self.last
            && *id == p.id
            && p.typ != eap::T_IDENTITY
        {
            return PeerOut::Send(last.clone());
        }
        match p.typ {
            eap::T_IDENTITY => {
                // a new authentication begins
                self.reset();
                let id = self.identity.clone();
                self.answer(p.id, Packet::identity_response(p.id, &id))
            }
            eap::T_NOTIFICATION => self.answer(p.id, Packet::response(p.id, eap::T_NOTIFICATION, &[])),
            eap::T_MD5 if self.method == Method::Md5 => {
                let Some(d) = Md5Data::parse(&p.data) else {
                    return PeerOut::Silent;
                };
                let value = eap::md5_value(p.id, self.password.as_bytes(), &d.value, crypto::md5_parts);
                self.finished = true;
                self.used = Some(Method::Md5);
                let data = Md5Data { value: value.to_vec(), name: self.identity.clone().into_bytes() };
                self.answer(p.id, Packet::response(p.id, eap::T_MD5, &data.to_bytes()))
            }
            eap::T_PWD if self.method == Method::Pwd => self.pwd_request(p, rnd),
            _ => {
                let own = self.method.typ();
                self.answer(p.id, Packet::nak(p.id, &[own]))
            }
        }
    }

    fn pwd_request(&mut self, p: &Packet, rnd: Rnd) -> PeerOut {
        let Some(x) = Pwd::parse(&p.data) else {
            return PeerOut::Silent;
        };
        match (x, &self.pwd) {
            (Pwd::Id(i), _) => {
                if i.group != eappwd::GROUP || i.rand_func != eappwd::RAND_FUNC || i.prf != eappwd::PRF || i.prep != 0 {
                    self.pwd = PwdPeer::Done;
                    return PeerOut::Failure;
                }
                self.pwd = PwdPeer::Commit { token: i.token, server: i.identity.clone() };
                let reply = PwdId { identity: self.identity.clone().into_bytes(), ..i };
                self.answer(p.id, Packet::response(p.id, eap::T_PWD, &Pwd::Id(reply).to_bytes()))
            }
            (Pwd::Commit { element, scalar }, PwdPeer::Commit { token, server }) => {
                let pwe = eappwd::pwe(token, self.identity.as_bytes(), server, self.password.as_bytes());
                let (rand, scal, elem) = draw_commit(&pwe, rnd);
                let Some(ss) = eappwd::shared(&pwe, &rand, &scalar, &element) else {
                    self.pwd = PwdPeer::Done;
                    return PeerOut::Failure;
                };
                self.pwd = PwdPeer::Confirm { ss, elem, scal, s_elem: element, s_scal: scalar };
                self.answer(p.id, Packet::response(p.id, eap::T_PWD, &Pwd::Commit { element: elem, scalar: scal }.to_bytes()))
            }
            (Pwd::Confirm(cs), PwdPeer::Confirm { ss, elem, scal, s_elem, s_scal }) => {
                let expect = eappwd::confirm(ss, s_elem, s_scal, elem, scal);
                if !crypto::ct_eq(&expect, &cs) {
                    self.pwd = PwdPeer::Done;
                    return PeerOut::Failure;
                }
                let cp = eappwd::confirm(ss, elem, scal, s_elem, s_scal);
                let (msk, emsk) = eappwd::keys(ss, &cp, &cs, scal, s_scal);
                self.msk = Some(msk);
                self.emsk = Some(emsk);
                self.finished = true;
                self.used = Some(Method::Pwd);
                self.pwd = PwdPeer::Done;
                self.answer(p.id, Packet::response(p.id, eap::T_PWD, &Pwd::Confirm(cp).to_bytes()))
            }
            _ => PeerOut::Silent,
        }
    }
}

/// what a server does with a packet
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ServerOut {
    /// send this Request
    Send(Vec<u8>),
    /// the peer is in: the EAP-Success, the keys of a method that makes them
    Success { eap: Vec<u8>, msk: Option<[u8; 64]>, emsk: Option<[u8; 64]> },
    /// the peer is out: the EAP-Failure
    Failure { eap: Vec<u8> },
    /// nothing to do
    Silent,
}

/// where the server's method is
#[derive(Clone, Debug)]
enum SrvState {
    Identity,
    Md5 { challenge: [u8; 16] },
    PwdId { token: [u8; 4] },
    PwdCommit { pwe: [u8; 64], rand: [u8; 32], scal: [u8; 32], elem: [u8; 64] },
    PwdConfirm { ss: [u8; 32], scal: [u8; 32], p_elem: [u8; 64], p_scal: [u8; 32], elem: [u8; 64], cs: [u8; 32] },
    Done,
}

/// the server's side of EAP
#[derive(Clone, Debug)]
pub struct Server {
    server_id: String,
    methods: Vec<Method>,
    /// the methods not offered yet
    next: usize,
    now: Option<Method>,
    user: String,
    id: u8,
    st: SrvState,
}

impl Server {
    pub fn new(server_id: &str, methods: &[Method]) -> Server {
        Server { server_id: String::from(server_id), methods: methods.to_vec(), next: 0, now: None, user: String::new(), id: 0, st: SrvState::Identity }
    }
    /// who is logging in (after the Response/Identity)
    pub fn user(&self) -> &str {
        &self.user
    }
    /// the method it runs now
    pub fn method(&self) -> Option<Method> {
        self.now
    }

    fn fail(&mut self) -> ServerOut {
        self.st = SrvState::Done;
        ServerOut::Failure { eap: Packet::failure(self.id.wrapping_add(1)).to_bytes() }
    }

    /// the Request of a method
    fn offer(&mut self, m: Method, rnd: Rnd) -> ServerOut {
        self.now = Some(m);
        self.id = self.id.wrapping_add(1);
        match m {
            Method::Md5 => {
                let mut challenge = [0u8; 16];
                rnd(&mut challenge);
                self.st = SrvState::Md5 { challenge };
                let data = Md5Data { value: challenge.to_vec(), name: self.server_id.clone().into_bytes() };
                ServerOut::Send(Packet::request(self.id, eap::T_MD5, &data.to_bytes()).to_bytes())
            }
            Method::Pwd => {
                let mut token = [0u8; 4];
                rnd(&mut token);
                self.st = SrvState::PwdId { token };
                let i = PwdId { group: eappwd::GROUP, rand_func: eappwd::RAND_FUNC, prf: eappwd::PRF, token, prep: 0, identity: self.server_id.clone().into_bytes() };
                ServerOut::Send(Packet::request(self.id, eap::T_PWD, &Pwd::Id(i).to_bytes()).to_bytes())
            }
        }
    }

    /// the first method not offered yet, or Failure
    fn offer_next(&mut self, wanted: Option<&[u8]>, rnd: Rnd) -> ServerOut {
        // a Nak names what the peer takes; else the methods in their order
        let pick = match wanted {
            Some(w) => self.methods.iter().copied().find(|m| w.contains(&m.typ()) && Some(*m) != self.now),
            None => self.methods.get(self.next).copied(),
        };
        self.next += 1;
        match pick {
            Some(m) => self.offer(m, rnd),
            None => self.fail(),
        }
    }

    /// a Response; `lookup` gives a user's password, `rnd` the random bytes
    pub fn on_packet(&mut self, b: &[u8], lookup: &dyn Fn(&str) -> Option<String>, rnd: Rnd) -> ServerOut {
        let Some(p) = Packet::parse(b) else {
            return ServerOut::Silent;
        };
        if p.code != eap::RESPONSE {
            return ServerOut::Silent;
        }
        if matches!(self.st, SrvState::Identity) {
            if p.typ != eap::T_IDENTITY {
                return ServerOut::Silent;
            }
            self.user = p.identity();
            self.id = p.id;
            if lookup(&self.user).is_none() {
                return self.fail();
            }
            return self.offer_next(None, rnd);
        }
        if p.id != self.id {
            return ServerOut::Silent;
        }
        if p.typ == eap::T_NAK {
            return self.offer_next(Some(&p.nak_types()), rnd);
        }
        let Some(pw) = lookup(&self.user) else {
            return self.fail();
        };
        match self.st.clone() {
            SrvState::Md5 { challenge } if p.typ == eap::T_MD5 => {
                let Some(d) = Md5Data::parse(&p.data) else {
                    return self.fail();
                };
                let want = eap::md5_value(p.id, pw.as_bytes(), &challenge, crypto::md5_parts);
                if !crypto::ct_eq(&d.value, &want) {
                    return self.fail();
                }
                self.st = SrvState::Done;
                ServerOut::Success { eap: Packet::success(self.id.wrapping_add(1)).to_bytes(), msk: None, emsk: None }
            }
            SrvState::PwdId { token } if p.typ == eap::T_PWD => {
                let Some(Pwd::Id(i)) = Pwd::parse(&p.data) else {
                    return self.fail();
                };
                if i.group != eappwd::GROUP || i.rand_func != eappwd::RAND_FUNC || i.prf != eappwd::PRF || i.token != token || i.prep != 0 {
                    return self.fail();
                }
                let pwe = eappwd::pwe(&token, &i.identity, self.server_id.as_bytes(), pw.as_bytes());
                let (rand, scal, elem) = draw_commit(&pwe, rnd);
                self.st = SrvState::PwdCommit { pwe, rand, scal, elem };
                self.id = self.id.wrapping_add(1);
                ServerOut::Send(Packet::request(self.id, eap::T_PWD, &Pwd::Commit { element: elem, scalar: scal }.to_bytes()).to_bytes())
            }
            SrvState::PwdCommit { pwe, rand, scal, elem } if p.typ == eap::T_PWD => {
                let Some(Pwd::Commit { element, scalar }) = Pwd::parse(&p.data) else {
                    return self.fail();
                };
                // a peer that reflects the server's own commit is refused (RFC 5931 2.8.5.2)
                if element == elem && scalar == scal {
                    return self.fail();
                }
                let Some(ss) = eappwd::shared(&pwe, &rand, &scalar, &element) else {
                    return self.fail();
                };
                let cs = eappwd::confirm(&ss, &elem, &scal, &element, &scalar);
                self.st = SrvState::PwdConfirm { ss, scal, p_elem: element, p_scal: scalar, elem, cs };
                self.id = self.id.wrapping_add(1);
                ServerOut::Send(Packet::request(self.id, eap::T_PWD, &Pwd::Confirm(cs).to_bytes()).to_bytes())
            }
            SrvState::PwdConfirm { ss, scal, p_elem, p_scal, elem, cs } if p.typ == eap::T_PWD => {
                let Some(Pwd::Confirm(cp)) = Pwd::parse(&p.data) else {
                    return self.fail();
                };
                let want = eappwd::confirm(&ss, &p_elem, &p_scal, &elem, &scal);
                if !crypto::ct_eq(&want, &cp) {
                    return self.fail();
                }
                let (msk, emsk) = eappwd::keys(&ss, &cp, &cs, &p_scal, &scal);
                self.st = SrvState::Done;
                ServerOut::Success { eap: Packet::success(self.id.wrapping_add(1)).to_bytes(), msk: Some(msk), emsk: Some(emsk) }
            }
            _ => self.fail(),
        }
    }
}
