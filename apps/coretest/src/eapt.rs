// eapt -- checks of netpkt::eap, EAPOL beside its keys, the EAP attributes of netpkt::radius, the
// group of crypto::p256, crypto::eappwd and the state machines of eapm: frames byte for byte, the
// vectors of an independent reference of EAP-PWD (a few lines of Python with plain affine
// formulas, the same token, names, password and random scalars), the MPPE keys of RFC 2548,
// peer and server in a whole run for both methods, a wrong password, Nak, repeated requests; EAPOL
// and EAP through the dissector.  IMAP's codec is checked here too (imapt below, group "eap").

use alloc::string::String;
use alloc::vec::Vec;

use crypto::{eappwd, p256};
use eapm::{Method, Peer, PeerOut, Server, ServerOut};
use netpkt::decode::{self, Proto};
use netpkt::eap::{self, Md5Data, Packet, Pwd, PwdId};
use netpkt::eapol::{self, EapolView};
use netpkt::eth::{EthHdr, ET_EAPOL};
use netpkt::radius::{self, Attr};
use netpkt::Mac;

use crate::T;

fn h(s: &str) -> Vec<u8> {
    crypto::unhex(&s.replace(' ', "")).expect("hex")
}

fn a32(s: &str) -> [u8; 32] {
    let mut a = [0u8; 32];
    a.copy_from_slice(&h(s));
    a
}

fn a64(s: &str) -> [u8; 64] {
    let mut a = [0u8; 64];
    a.copy_from_slice(&h(s));
    a
}

const PWE: &str = "bdfaa1b01ef6b4aea60b5e95bbf5ae271493085fe2fde534ce26a0107addb4616f7445063ed95fa4df1e712f8e9b6ce4a37098196de7d072b87e00d4c7980468";
const RAND_S: &str = "50545e556f49bdae3f444822047794aaa5d665ec61136a947bda89363e58e47a";
const MASK_S: &str = "7d81c9001ac69a7cbd3f324b2ed4df2e0034844462634bb27b281c64757c0907";
const RAND_P: &str = "dee9f473001a721f1f079e390788d528f328e550ab0b524fe4beba9a403da613";
const MASK_P: &str = "3dbcd202930583222dd1dc512c7e08ab16f788a6785bf928e9a5ac08aa8438ef";
const SCALAR_S: &str = "cdd627558a10582afc837a6d334c73d8a60aea30c376b646f702a59ab3d4ed81";
const ELEMENT_S: &str = "21863680157b816653daefbf1613b3171b3ee2f84f54fe6c89ec7b9b30c95ad5c6edb1906f43fa5b20dc4dc0e3e40e0511b43a60826b51af62346cc144685b63";
const SCALAR_P: &str = "1ca6c676931ff5404cd97a8a3406ddd44d3973497c4facf3daaa9bdfee5eb9b1";
const ELEMENT_P: &str = "6e466aac1c582f1ce9de1ca4c48392ffea9ad066298946e1d05865e127c0e6adc6d3de8468cd94bc9de3a720698391fdc9bdce85d4079d8fc0c24390758ebb5f";
const SS: &str = "36093d8fc66c99b6c393c278bc999daf20d89801829fa199787af944f64a5799";
const CONFIRM_S: &str = "e55f6279b8f2bf4547c6aa24e9595d5a72ed1bfb3b2b0d3fc72c8f5f297b8d4e";
const CONFIRM_P: &str = "a22902b321f1d825d5c01e308a7d43430e5eb01f5cf4305ea263ab7007601e83";
const MSK: &str = "12ba27ad86b78c121c97b0e3ee2ddb99fc336998c95bd7314d945c6497a73e2e750d0813a47f0a12172b296eaba8fbbbaf7bfc9c641994fccc339a73cb211878";
const EMSK: &str = "a97fe9a77ef60903ecf59b38c43c6802680c0a6b6b55e3f58e8958a5ec34e10b031e3e149146ee2bc1aed0d6e4c1febbd0f98126e448541418345d62b32b4c6e";
const TWO_G: &str = "7cf27b188d034f7e8a52380304b51ac3c08969e277f21b35a60b48fc4766997807775510db8ed040293d9ac69f7430dbba7dade63ce982299e04b79d227873d1";
const G: &str = "6b17d1f2e12c4247f8bce6e563a440f277037d812deb33a0f4a13945d898c2964fe342e2fe1a7f9b8ee7eb4a7c0f9e162bce33576b315ececbb6406837bf51f5";
const MD5_VALUE: &str = "758a0228e85e5544d74843dd0633f99e";
const MPPE: &str = "81239d67907a68e08684ad0573d02da890089910f938ae31abd8f3e4abf7e23e3daab481c0129eb5fdd8710a02d845bfd9fa";

/// a deterministic source of random bytes
fn counter_rnd(seed: u8) -> impl FnMut(&mut [u8]) {
    let mut n = seed;
    move |o: &mut [u8]| {
        for b in o.iter_mut() {
            n = n.wrapping_mul(167).wrapping_add(13);
            *b = n;
        }
    }
}

pub fn run(t: &mut T) {
    eap_codec(t);
    eapol_frames(t);
    radius_attrs(t);
    group(t);
    pwd_vectors(t);
    runs(t);
    crate::imapt::run(t);
}

fn eap_codec(t: &mut T) {
    // Request/Identity: 01 05 00 05 01
    let r = Packet::identity_request(5, "");
    t.eq("request identity", r.to_bytes(), h("01 05 00 05 01"));
    let id = Packet::identity_response(5, "anna");
    t.eq("response identity", id.to_bytes(), h("02 05 00 09 01 61 6e 6e 61"));
    t.eq("parse", Packet::parse(&id.to_bytes()), Some(id.clone()));
    t.eq("identity text", id.identity(), String::from("anna"));
    t.eq("success", Packet::success(9).to_bytes(), h("03 09 00 04"));
    t.eq("failure", Packet::failure(9).to_bytes(), h("04 09 00 04"));
    let mut padded = Packet::success(2).to_bytes();
    padded.extend_from_slice(&[0; 40]);
    t.eq("padding goes", Packet::parse(&padded), Some(Packet::success(2)));
    t.eq("too long", Packet::parse(&h("01 01 00 20 01")), None);
    t.eq("no type", Packet::parse(&h("01 01 00 04")), None);
    t.eq("short", Packet::parse(&h("01 01 00")), None);
    let nak = Packet::nak(6, &[eap::T_PWD]);
    t.eq("nak", nak.to_bytes(), h("02 06 00 06 03 34"));
    t.eq("nak types", nak.nak_types(), alloc::vec![eap::T_PWD]);
    t.eq("nak none", Packet::nak(6, &[]).to_bytes(), h("02 06 00 06 03 00"));
    // EAP-MD5
    let d = Md5Data { value: alloc::vec![1, 2, 3], name: b"sw".to_vec() };
    t.eq("md5 data", d.to_bytes(), h("03 01 02 03 73 77"));
    t.eq("md5 parse", Md5Data::parse(&d.to_bytes()), Some(d));
    t.eq("md5 too short", Md5Data::parse(&h("10 01")), None);
    let ch: Vec<u8> = (0..16).collect();
    t.eq("md5 value", eap::md5_value(7, b"geheim", &ch, crypto::md5_parts).to_vec(), h(MD5_VALUE));
    // EAP-PWD
    let i = PwdId { group: 19, rand_func: 1, prf: 1, token: [1, 2, 3, 4], prep: 0, identity: b"radius".to_vec() };
    let b = Pwd::Id(i.clone()).to_bytes();
    t.eq("pwd id", b.clone(), h("01 00 13 01 01 01 02 03 04 00 72 61 64 69 75 73"));
    t.eq("pwd id parse", Pwd::parse(&b), Some(Pwd::Id(i)));
    let c = Pwd::Commit { element: [7; 64], scalar: [8; 32] };
    t.eq("pwd commit", Pwd::parse(&c.to_bytes()), Some(c.clone()));
    t.eq("pwd commit size", c.to_bytes().len(), 97);
    t.eq("pwd confirm", Pwd::parse(&Pwd::Confirm([9; 32]).to_bytes()), Some(Pwd::Confirm([9; 32])));
    t.eq("a fragment is refused", Pwd::parse(&[0x80 | eap::PWD_CONFIRM, 1, 2]), None);
    t.eq("a short commit is refused", Pwd::parse(&[eap::PWD_COMMIT, 1, 2]), None);
    // one line about a packet
    t.eq("info request", eap::info(&Packet::identity_request(1, "").to_bytes()), String::from("Request, Identity"));
    t.eq("info response", eap::info(&Packet::identity_response(1, "bob").to_bytes()), String::from("Response, Identity (bob)"));
    t.eq("info success", eap::info(&Packet::success(1).to_bytes()), String::from("Success"));
    t.eq("info nak", eap::info(&Packet::nak(1, &[eap::T_MD5]).to_bytes()), String::from("Response, Nak (MD5-Challenge)"));
    t.eq("info pwd", eap::info(&Packet::request(1, eap::T_PWD, &Pwd::Confirm([0; 32]).to_bytes()).to_bytes()), String::from("Request, PWD-Confirm"));
    t.eq("info bad", eap::info(&[1]), String::from("EAP (malformed)"));
}

fn eapol_frames(t: &mut T) {
    t.eq("start", eapol::start(2), h("02 01 00 00"));
    t.eq("logoff", eapol::logoff(1), h("01 02 00 00"));
    let e = Packet::identity_request(1, "").to_bytes();
    let pdu = eapol::eap_pdu(2, &e);
    t.eq("eap pdu", pdu.clone(), h("02 00 00 05 01 01 00 05 01"));
    let v = EapolView::parse(&pdu).expect("eapol");
    t.eq("kind", v.kind(), eapol::T_EAP);
    t.eq("body", v.body().to_vec(), e);
    // the dissector: an EAP layer under EAPOL in a padded frame
    let frame = |pdu: &[u8]| {
        let mut f = EthHdr { dst: Mac(eapol::PAE_GROUP), src: Mac([2, 0, 0, 0, 0, 1]), ethertype: ET_EAPOL }.frame(pdu);
        netpkt::eth::pad(&mut f);
        f
    };
    let f = frame(&pdu);
    t.eq("frame size", f.len(), 60);
    t.check("mask", decode::proto_mask(&f) & decode::P_EAPOL != 0);
    let (top, info) = decode::info(&f);
    t.eq("top protocol", top, Proto::Eap);
    t.eq("summary", info, String::from("Request, Identity"));
    let layers = decode::layers(&f);
    let eap_layer = layers.iter().find(|l| l.proto == Proto::Eap);
    t.check("EAP layer", eap_layer.is_some());
    if let Some(l) = eap_layer {
        t.eq("EAP title", l.title.as_str(), "Extensible Authentication Protocol, Request, Identity");
        t.check("EAP code field", l.fields.iter().any(|x| x.name == "Code" && x.value == "Request (1)"));
        t.check("EAP type field", l.fields.iter().any(|x| x.name == "Type" && x.value == "Identity (1)"));
    }
    // EAPOL-Start has no EAP layer, its summary is the kind
    let f = frame(&eapol::start(2));
    let (top, info) = decode::info(&f);
    t.eq("start top", top, Proto::Eapol);
    t.eq("start summary", info, String::from("EAPOL-Start"));
    // EAP-PWD's commit field by field
    let f = frame(&eapol::eap_pdu(2, &Packet::request(2, eap::T_PWD, &Pwd::Commit { element: [7; 64], scalar: [8; 32] }.to_bytes()).to_bytes()));
    let layers = decode::layers(&f);
    let l = layers.iter().find(|l| l.proto == Proto::Eap);
    t.check("commit element", l.is_some_and(|l| l.fields.iter().any(|x| x.name == "Element")));
    t.check("commit scalar", l.is_some_and(|l| l.fields.iter().any(|x| x.name == "Scalar")));
    // the AKM names of enterprise networks
    let k = netpkt::eapol::Rsne { akm: eapol::AKM_8021X, caps: 0 }.to_bytes();
    t.eq("rsne akm", netpkt::eapol::Rsne::parse(&k).map(|r| r.akm), Some(eapol::AKM_8021X));
}

fn radius_attrs(t: &mut T) {
    let md5: radius::Md5Fn = crypto::md5_parts;
    // EAP-Message: 600 bytes in pieces of 253
    let mut p = radius::Packet::new(radius::ACCESS_REQUEST, 1, [0; 16]);
    let eap: Vec<u8> = (0..600u32).map(|i| i as u8).collect();
    p.push_eap(&eap);
    t.eq("pieces", p.all(radius::A_EAP_MESSAGE).len(), 3);
    t.eq("first piece", p.all(radius::A_EAP_MESSAGE)[0].value.len(), 253);
    t.eq("back together", p.eap(), Some(eap));
    t.eq("no EAP", radius::Packet::new(radius::ACCESS_ACCEPT, 1, [0; 16]).eap(), None);
    // with a Message-Authenticator as RFC 3579 asks
    let mut q = radius::Packet::new(radius::ACCESS_REQUEST, 2, [3; 16]);
    q.push_eap(&Packet::identity_response(1, "anna").to_bytes());
    q.push(Attr::message_auth());
    let mut b = q.to_bytes();
    t.check("set", radius::set_message_auth(&mut b, None, b"s3", crypto::hmac_md5));
    t.eq("right secret", radius::check_message_auth(&b, None, b"s3", crypto::hmac_md5), Some(true));
    t.eq("wrong secret", radius::check_message_auth(&b, None, b"xx", crypto::hmac_md5), Some(false));
    t.check("EAP-Message shows", radius::value_text(radius::A_EAP_MESSAGE, &Packet::success(1).to_bytes()) == "Success");
    // State
    q.push(Attr::bytes(radius::A_STATE, &[1, 2, 3]));
    t.eq("state", q.attr(radius::A_STATE).map(|a| a.value.clone()), Some(alloc::vec![1, 2, 3]));
    // VLAN assignment
    let mut a = radius::Packet::new(radius::ACCESS_ACCEPT, 2, [0; 16]);
    a.push_vlan(20);
    t.eq("vlan", a.vlan(), Some(20));
    t.eq("tunnel type bytes", a.attr(radius::A_TUNNEL_TYPE).map(|x| x.value.clone()), Some(h("00 00 00 0d")));
    t.eq("tunnel medium bytes", a.attr(radius::A_TUNNEL_MEDIUM_TYPE).map(|x| x.value.clone()), Some(h("00 00 00 06")));
    t.eq("group text", radius::value_text(radius::A_TUNNEL_PRIVATE_GROUP_ID, b"20"), String::from("\"20\""));
    t.eq("type text", radius::value_text(radius::A_TUNNEL_TYPE, &h("00 00 00 0d")), String::from("VLAN (13)"));
    t.eq("name", radius::attr_name(radius::A_TUNNEL_PRIVATE_GROUP_ID), String::from("Tunnel-Private-Group-ID"));
    // a tag byte in front of the values (other servers write one)
    let mut tagged = radius::Packet::new(radius::ACCESS_ACCEPT, 3, [0; 16]);
    tagged.push(Attr::bytes(radius::A_TUNNEL_TYPE, &h("01 00 00 0d")));
    tagged.push(Attr::bytes(radius::A_TUNNEL_MEDIUM_TYPE, &h("01 00 00 06")));
    tagged.push(Attr::bytes(radius::A_TUNNEL_PRIVATE_GROUP_ID, b"\x0130"));
    t.eq("tagged vlan", tagged.vlan(), Some(30));
    let mut none = radius::Packet::new(radius::ACCESS_ACCEPT, 3, [0; 16]);
    none.push(Attr::text(radius::A_TUNNEL_PRIVATE_GROUP_ID, "20"));
    t.eq("no type: no vlan", none.vlan(), None);
    // MS-MPPE keys: the reference's bytes, both ways
    let key: Vec<u8> = (0..32).collect();
    let mut auth = [0u8; 16];
    for (i, x) in auth.iter_mut().enumerate() {
        *x = 16 + i as u8;
    }
    let sealed = radius::mppe_seal(&key, b"testing123", &auth, 0x0123, md5);
    t.eq("mppe bytes", sealed.clone(), h(MPPE));
    t.eq("mppe open", radius::mppe_open(&sealed, b"testing123", &auth, md5), Some(key));
    let wrong = radius::mppe_open(&sealed, b"other", &auth, md5);
    t.check("mppe wrong secret", wrong.as_deref() != Some(&sealed[..]));
    // an MSK through both attributes
    let msk = a64(MSK);
    let mut acc = radius::Packet::new(radius::ACCESS_ACCEPT, 4, [0; 16]);
    acc.push_msk(&msk, b"testing123", &auth, [1, 2], md5);
    t.eq("two attributes", acc.all(radius::A_VENDOR_SPECIFIC).len(), 2);
    t.eq("the MSK comes back", acc.msk(b"testing123", &auth, md5), Some(msk));
    t.eq("no MSK without them", radius::Packet::new(radius::ACCESS_ACCEPT, 4, [0; 16]).msk(b"x", &auth, md5), None);
    let vsa = acc.all(radius::A_VENDOR_SPECIFIC)[0].value.clone();
    t.check("vsa hidden in the dissector", radius::value_text(radius::A_VENDOR_SPECIFIC, &vsa).starts_with("MS-MPPE-Recv-Key (hidden"));
}

fn group(t: &mut T) {
    let g = a64(G);
    t.check("G on curve", p256::on_curve(&g));
    let mut bad = g;
    bad[63] ^= 1;
    t.check("off curve", !p256::on_curve(&bad));
    let mut two = [0u8; 32];
    two[31] = 2;
    t.eq("2G", p256::mul_point(&two, &g).map(|p| p.to_vec()), Some(h(TWO_G)));
    t.eq("G + G", p256::add_points(&g, &g).map(|p| p.to_vec()), Some(h(TWO_G)));
    let mut x = [0u8; 32];
    x.copy_from_slice(&g[..32]);
    let mut y = [0u8; 32];
    y.copy_from_slice(&g[32..]);
    t.eq("y of x", p256::y_for_x(&x, y[31] & 1 == 1), Some(y));
    let other = p256::y_for_x(&x, y[31] & 1 == 0).expect("other y");
    t.check("the other y", other != y && other[31] & 1 != y[31] & 1);
    let neg = p256::neg_point(&g);
    t.eq("-G has the other y", neg[32..].to_vec(), other.to_vec());
    t.eq("G + -G is the identity", p256::add_points(&g, &neg), None);
    let two_g = a64(TWO_G);
    t.eq("2G + -G", p256::add_points(&two_g, &neg).map(|p| p.to_vec()), Some(g.to_vec()));
    // x with no point: 5 has no y on this curve, or the check would not hold for both
    let mut xs = 0;
    for i in 1..8u8 {
        let mut c = [0u8; 32];
        c[31] = i;
        if p256::y_for_x(&c, false).is_some() {
            xs += 1;
        }
    }
    t.check("not every x has a point", (1..7).contains(&xs));
    t.check("x at p", !p256::below_p(&[0xFF; 32]));
    // scalars
    let n = a32("ffffffff00000000ffffffffffffffffbce6faada7179e84f3b9cac2fc632551");
    let mut n1 = n;
    n1[31] -= 1;
    let mut one = [0u8; 32];
    one[31] = 1;
    t.eq("n - 1 + 1 = 0", p256::scalar_add(&n1, &one), [0u8; 32]);
    t.eq("n - 1 + 2 = 1", p256::scalar_add(&n1, &two), one);
    t.check("0 is no scalar", !p256::scalar_ok(&[0; 32]));
    t.check("n is no scalar", !p256::scalar_ok(&n));
    t.check("n - 1 is one", p256::scalar_ok(&n1));
}

fn pwd_vectors(t: &mut T) {
    let token = [1u8, 2, 3, 4];
    let pwe = eappwd::pwe(&token, b"anna", b"radius", b"geheim");
    t.eq("pwe", pwe.to_vec(), h(PWE));
    t.check("pwe on curve", p256::on_curve(&pwe));
    let other = eappwd::pwe(&token, b"anna", b"radius", b"falsch");
    t.check("another password: another pwe", other != pwe);
    let (rs, ms, rp, mp) = (a32(RAND_S), a32(MASK_S), a32(RAND_P), a32(MASK_P));
    let (ss_s, es) = eappwd::commit(&pwe, &rs, &ms).expect("commit");
    let (ss_p, ep) = eappwd::commit(&pwe, &rp, &mp).expect("commit");
    t.eq("scalar s", ss_s.to_vec(), h(SCALAR_S));
    t.eq("element s", es.to_vec(), h(ELEMENT_S));
    t.eq("scalar p", ss_p.to_vec(), h(SCALAR_P));
    t.eq("element p", ep.to_vec(), h(ELEMENT_P));
    let k1 = eappwd::shared(&pwe, &rs, &ss_p, &ep).expect("ss");
    let k2 = eappwd::shared(&pwe, &rp, &ss_s, &es).expect("ss");
    t.eq("ss", k1.to_vec(), h(SS));
    t.eq("both sides", k2, k1);
    let cs = eappwd::confirm(&k1, &es, &ss_s, &ep, &ss_p);
    let cp = eappwd::confirm(&k1, &ep, &ss_p, &es, &ss_s);
    t.eq("confirm s", cs.to_vec(), h(CONFIRM_S));
    t.eq("confirm p", cp.to_vec(), h(CONFIRM_P));
    let (msk, emsk) = eappwd::keys(&k1, &cp, &cs, &ss_p, &ss_s);
    t.eq("msk", msk.to_vec(), h(MSK));
    t.eq("emsk", emsk.to_vec(), h(EMSK));
    t.eq("ciphersuite", eappwd::ciphersuite(), [0, 19, 1, 1]);
    // what commit and shared refuse
    t.eq("zero rand", eappwd::commit(&pwe, &[0; 32], &ms), None);
    let mut bad = ep;
    bad[63] ^= 1;
    t.eq("element off the curve", eappwd::shared(&pwe, &rs, &ss_p, &bad), None);
    t.eq("scalar zero", eappwd::shared(&pwe, &rs, &[0; 32], &ep), None);
    t.eq("kdf length", eappwd::kdf(b"k", b"l", 100).len(), 100);
}

/// a whole exchange between a peer and a server; the outcome and the packets on the way
fn exchange(peer: &mut Peer, srv: &mut Server, users: &[(&str, &str)], seed: u8) -> (Option<ServerOut>, PeerOut, usize) {
    let mut pr = counter_rnd(seed);
    let mut sr = counter_rnd(seed.wrapping_add(77));
    let lookup = |n: &str| users.iter().find(|u| u.0 == n).map(|u| String::from(u.1));
    let mut req = Packet::identity_request(1, "").to_bytes();
    let mut n = 0;
    loop {
        n += 1;
        let out = peer.on_packet(&req, &mut pr);
        let PeerOut::Send(resp) = out else {
            return (None, out, n);
        };
        match srv.on_packet(&resp, &lookup, &mut sr) {
            ServerOut::Send(r) => req = r,
            ServerOut::Silent => return (Some(ServerOut::Silent), PeerOut::Silent, n),
            done @ ServerOut::Success { .. } => {
                let ServerOut::Success { eap, .. } = &done else { unreachable!() };
                let last = peer.on_packet(eap, &mut pr);
                return (Some(done), last, n);
            }
            done @ ServerOut::Failure { .. } => {
                let ServerOut::Failure { eap } = &done else { unreachable!() };
                let last = peer.on_packet(eap, &mut pr);
                return (Some(done), last, n);
            }
        }
    }
}

fn runs(t: &mut T) {
    let users = [("anna", "geheim"), ("bob", "x")];
    // EAP-MD5: Identity, Challenge
    let mut p = Peer::new("anna", "geheim", Method::Md5);
    let mut s = Server::new("radius", &[Method::Md5, Method::Pwd]);
    let (out, last, n) = exchange(&mut p, &mut s, &users, 1);
    t.check("md5 success", matches!(out, Some(ServerOut::Success { msk: None, .. })));
    t.eq("md5 peer", last, PeerOut::Success);
    t.eq("md5 rounds", n, 2);
    t.eq("md5 no keys", p.msk(), None);
    t.eq("md5 method", p.method_used(), Some(Method::Md5));
    t.eq("server's user", s.user(), "anna");
    // the wrong password
    let mut p = Peer::new("anna", "falsch", Method::Md5);
    let mut s = Server::new("radius", &[Method::Md5]);
    let (out, last, _) = exchange(&mut p, &mut s, &users, 2);
    t.check("md5 failure", matches!(out, Some(ServerOut::Failure { .. })));
    t.eq("md5 peer failure", last, PeerOut::Failure);
    // a user nobody knows
    let mut p = Peer::new("eve", "x", Method::Md5);
    let mut s = Server::new("radius", &[Method::Md5]);
    let (out, _, n) = exchange(&mut p, &mut s, &users, 3);
    t.check("unknown user", matches!(out, Some(ServerOut::Failure { .. })));
    t.eq("at once", n, 1);
    // EAP-PWD with a Nak of MD5: the peer asks for its method
    let mut p = Peer::new("anna", "geheim", Method::Pwd);
    let mut s = Server::new("radius", &[Method::Md5, Method::Pwd]);
    let (out, last, n) = exchange(&mut p, &mut s, &users, 4);
    let Some(ServerOut::Success { msk: Some(m), emsk: Some(e), .. }) = out else {
        t.check("pwd success", false);
        return;
    };
    t.eq("pwd peer", last, PeerOut::Success);
    t.eq("pwd rounds: identity, nak, id, commit, confirm", n, 5);
    t.eq("the same MSK", p.msk().copied(), Some(m));
    t.eq("the same EMSK", p.emsk().copied(), Some(e));
    t.eq("pwd method", p.method_used(), Some(Method::Pwd));
    // the same run again gives the same bytes
    let mut p2 = Peer::new("anna", "geheim", Method::Pwd);
    let mut s2 = Server::new("radius", &[Method::Md5, Method::Pwd]);
    let (_, _, _) = exchange(&mut p2, &mut s2, &users, 4);
    t.eq("deterministic", p2.msk().copied(), Some(m));
    // EAP-PWD with the wrong password: the peer's check of Confirm ends it, no answer
    let mut p = Peer::new("anna", "falsch", Method::Pwd);
    let mut s = Server::new("radius", &[Method::Pwd]);
    let (out, last, _) = exchange(&mut p, &mut s, &users, 5);
    t.check("pwd no server outcome", out.is_none());
    t.eq("pwd ends at the peer", last, PeerOut::Failure);
    t.eq("no keys", p.msk(), None);
    // a server with no method the peer takes
    let mut p = Peer::new("anna", "geheim", Method::Md5);
    let mut s = Server::new("radius", &[Method::Pwd]);
    let (out, last, _) = exchange(&mut p, &mut s, &users, 6);
    t.check("no common method", matches!(out, Some(ServerOut::Failure { .. })));
    t.eq("peer told", last, PeerOut::Failure);
    // a Success before anything ran is ignored, Notification is answered, a repeat too
    let mut rnd = counter_rnd(7);
    let mut p = Peer::new("anna", "geheim", Method::Md5);
    t.eq("early success", p.on_packet(&Packet::success(1).to_bytes(), &mut rnd), PeerOut::Silent);
    t.eq("notification", p.on_packet(&Packet::request(2, eap::T_NOTIFICATION, b"hi").to_bytes(), &mut rnd), PeerOut::Send(Packet::response(2, eap::T_NOTIFICATION, &[]).to_bytes()));
    let first = p.on_packet(&Packet::identity_request(3, "").to_bytes(), &mut rnd);
    t.check("identity answered", matches!(first, PeerOut::Send(_)));
    let ch = Packet::request(4, eap::T_MD5, &Md5Data { value: alloc::vec![1; 16], name: Vec::new() }.to_bytes()).to_bytes();
    let a = p.on_packet(&ch, &mut rnd);
    let b = p.on_packet(&ch, &mut rnd);
    t.eq("a repeat gets the same", a, b);
    t.eq("garbage", p.on_packet(&[1, 2], &mut rnd), PeerOut::Silent);
    // the words of a file
    t.eq("word", Method::parse("pwd"), Some(Method::Pwd));
    t.eq("word 2", Method::parse("EAP-MD5"), Some(Method::Md5));
    t.eq("word 3", Method::parse("PEAP"), None);
    t.check("keys", Method::Pwd.makes_keys() && !Method::Md5.makes_keys());
}
