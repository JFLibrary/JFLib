// eapol -- EAPOL frames (IEEE 802.1X-2004 clause 11) with the EAPOL-Key body of IEEE 802.11-2020
// 12.7.2: the four messages of the 4-way handshake, the RSN element a station and an access
// point put into them, and the key data elements (KDEs) of message 3.  No cryptography here:
// the caller computes the MIC over mic_input() (HMAC-SHA1-128 or AES-128-CMAC by the key
// descriptor version) and wraps the key data (the AES key wrap of RFC 3394)
//
//   ET_EAPOL (eth.rs)              0x888E
//   EapolView::parse(pdu)          version, packet type, body length, body
//   Key { info, key_len, replay, nonce, iv, rsc, mic, data }     the EAPOL-Key body
//   Key::parse(body) / Key::pdu(version) -> Vec<u8>   read one, write the whole EAPOL PDU
//   Key::message() -> Option<u8>   1 .. 4 of the 4-way handshake by its Key Information bits
//   mic_input(pdu) -> Vec<u8>      the PDU with the MIC field zero (what the MIC covers)
//   set_mic(pdu, mic)              puts the MIC into a written PDU
//   Rsne { akm, caps }             the RSN element (CCMP for group and pairwise, one AKM):
//                                  to_bytes / parse
//   kde_gtk(id, gtk) / kde_igtk(id, ipn, igtk) / kdes(data)   KDEs and the elements of key
//                                  data (with the padding of 12.7.2 j: 0xdd 0x00 ...)
//   pad_key_data(data)             the padding for the key wrap (a multiple of 8, at least 16)
//   KI_* (Key Information), AKM_* and CIPHER_CCMP (the suites of 00-0F-AC)
//
// The fields are big endian; the key replay counter is a 64-bit number.  Lengths are checked
// once by parse; a body shorter than the 95 fixed bytes is no Key.

use alloc::vec::Vec;

use crate::{be16, put16};

/// EAPOL packet types (802.1X-2004 11.3.2)
pub const T_EAP: u8 = 0;
pub const T_START: u8 = 1;
pub const T_LOGOFF: u8 = 2;
pub const T_KEY: u8 = 3;

/// the version 802.1X-2004 writes
pub const VERSION_2004: u8 = 2;

/// the RSN key descriptor type (802.11-2020 12.7.2)
pub const DESC_RSN: u8 = 2;

/// Key Information bits (802.11-2020 Figure 12-33)
pub const KI_VERSION: u16 = 0x0007;
pub const KI_PAIRWISE: u16 = 0x0008;
pub const KI_INSTALL: u16 = 0x0040;
pub const KI_ACK: u16 = 0x0080;
pub const KI_MIC: u16 = 0x0100;
pub const KI_SECURE: u16 = 0x0200;
pub const KI_ERROR: u16 = 0x0400;
pub const KI_REQUEST: u16 = 0x0800;
pub const KI_ENCRYPTED: u16 = 0x1000;

/// key descriptor versions: 2 HMAC-SHA1-128 MIC with the AES key wrap (AKM 2, PSK), 3 AES-128-CMAC
/// (AKM 6), 0 what the AKM defines (AKM 8, SAE: AES-128-CMAC)
pub const KDV_AKM: u16 = 0;
pub const KDV_SHA1: u16 = 2;
pub const KDV_CMAC: u16 = 3;

/// the fixed part of an EAPOL-Key body with a 16-byte MIC
pub const KEY_FIXED: usize = 95;
/// where the MIC sits in the body, its length
pub const MIC_OFF: usize = 77;
pub const MIC_LEN: usize = 16;

/// the OUI of IEEE 802.11 suites and KDEs
pub const OUI: [u8; 3] = [0x00, 0x0F, 0xAC];
/// cipher suite: CCMP-128
pub const CIPHER_CCMP: u8 = 4;
/// AKM suites: PSK (WPA2-Personal), PSK-SHA256, SAE (WPA3-Personal)
pub const AKM_PSK: u8 = 2;
pub const AKM_PSK_SHA256: u8 = 6;
pub const AKM_SAE: u8 = 8;
/// RSN capabilities: management frame protection capable / required
pub const RSN_MFPR: u16 = 0x0040;
pub const RSN_MFPC: u16 = 0x0080;

/// the element id of the RSN element, the id of a vendor element (KDEs)
pub const EID_RSN: u8 = 48;
pub const EID_VENDOR: u8 = 0xDD;
/// KDE types (802.11-2020 Table 12-10)
pub const KDE_GTK: u8 = 1;
pub const KDE_PMKID: u8 = 4;
pub const KDE_IGTK: u8 = 9;

/// the EAPOL header and body of a PDU
#[derive(Clone, Copy, Debug)]
pub struct EapolView<'a> {
    b: &'a [u8],
}

impl<'a> EapolView<'a> {
    /// the 4-byte header and a body as long as it says
    pub fn parse(b: &'a [u8]) -> Option<EapolView<'a>> {
        if b.len() < 4 || b.len() < 4 + be16(b, 2) as usize {
            return None;
        }
        Some(EapolView { b })
    }
    pub fn version(&self) -> u8 {
        self.b[0]
    }
    pub fn kind(&self) -> u8 {
        self.b[1]
    }
    pub fn body_len(&self) -> usize {
        be16(self.b, 2) as usize
    }
    pub fn body(&self) -> &'a [u8] {
        &self.b[4..4 + self.body_len()]
    }
    /// header and body without what follows (Ethernet padding)
    pub fn pdu(&self) -> &'a [u8] {
        &self.b[..4 + self.body_len()]
    }
}

/// the name of a packet type ("EAPOL-Key")
pub fn kind_name(t: u8) -> &'static str {
    match t {
        T_EAP => "EAP Packet",
        T_START => "EAPOL-Start",
        T_LOGOFF => "EAPOL-Logoff",
        T_KEY => "EAPOL-Key",
        4 => "EAPOL-Encapsulated-ASF-Alert",
        _ => "Unknown",
    }
}

/// an EAPOL-Key body (RSN descriptor, a 16-byte MIC)
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Key {
    pub info: u16,
    pub key_len: u16,
    pub replay: u64,
    pub nonce: [u8; 32],
    pub iv: [u8; 16],
    pub rsc: [u8; 8],
    pub mic: [u8; 16],
    pub data: Vec<u8>,
}

impl Key {
    /// a key body; None when it is shorter than its fixed part or its key data, or is no RSN
    /// descriptor
    pub fn parse(body: &[u8]) -> Option<Key> {
        if body.len() < KEY_FIXED || body[0] != DESC_RSN {
            return None;
        }
        let dlen = be16(body, 93) as usize;
        if body.len() < KEY_FIXED + dlen {
            return None;
        }
        let mut k = Key { info: be16(body, 1), key_len: be16(body, 3), ..Key::default() };
        let mut r = [0u8; 8];
        r.copy_from_slice(&body[5..13]);
        k.replay = u64::from_be_bytes(r);
        k.nonce.copy_from_slice(&body[13..45]);
        k.iv.copy_from_slice(&body[45..61]);
        k.rsc.copy_from_slice(&body[61..69]);
        k.mic.copy_from_slice(&body[MIC_OFF..MIC_OFF + MIC_LEN]);
        k.data = body[KEY_FIXED..KEY_FIXED + dlen].to_vec();
        Some(k)
    }
    /// the body bytes
    pub fn body(&self) -> Vec<u8> {
        let mut b = alloc::vec![0u8; KEY_FIXED + self.data.len()];
        b[0] = DESC_RSN;
        put16(&mut b, 1, self.info);
        put16(&mut b, 3, self.key_len);
        b[5..13].copy_from_slice(&self.replay.to_be_bytes());
        b[13..45].copy_from_slice(&self.nonce);
        b[45..61].copy_from_slice(&self.iv);
        b[61..69].copy_from_slice(&self.rsc);
        // 69..77: reserved (Key ID)
        b[MIC_OFF..MIC_OFF + MIC_LEN].copy_from_slice(&self.mic);
        put16(&mut b, 93, self.data.len() as u16);
        b[KEY_FIXED..].copy_from_slice(&self.data);
        b
    }
    /// the whole EAPOL PDU (header with `version`, packet type EAPOL-Key, the body)
    pub fn pdu(&self, version: u8) -> Vec<u8> {
        let body = self.body();
        let mut v = alloc::vec![version, T_KEY, 0, 0];
        put16(&mut v, 2, body.len() as u16);
        v.extend_from_slice(&body);
        v
    }
    /// the key descriptor version (KDV_*)
    pub fn version(&self) -> u16 {
        self.info & KI_VERSION
    }
    pub fn has(&self, bits: u16) -> bool {
        self.info & bits == bits
    }
    /// which message of the 4-way handshake: 1 (Ack), 2 (MIC), 3 (Ack, MIC, Install), 4 (MIC,
    /// Secure); None for a group key message or a request
    pub fn message(&self) -> Option<u8> {
        if !self.has(KI_PAIRWISE) || self.has(KI_REQUEST) {
            return None;
        }
        let (ack, mic, install, secure) = (self.has(KI_ACK), self.has(KI_MIC), self.has(KI_INSTALL), self.has(KI_SECURE));
        match (ack, mic) {
            (true, false) => Some(1),
            (true, true) if install => Some(3),
            (false, true) if secure && self.nonce == [0; 32] => Some(4),
            (false, true) if !secure || !self.data.is_empty() => Some(2),
            (false, true) => Some(4),
            _ => None,
        }
    }
}

/// the PDU with the MIC field set to zero: what the MIC covers (None when it holds no key
/// body)
pub fn mic_input(pdu: &[u8]) -> Option<Vec<u8>> {
    let v = EapolView::parse(pdu)?;
    if v.kind() != T_KEY || v.body_len() < KEY_FIXED {
        return None;
    }
    let mut out = v.pdu().to_vec();
    for b in &mut out[4 + MIC_OFF..4 + MIC_OFF + MIC_LEN] {
        *b = 0;
    }
    Some(out)
}

/// writes the MIC into a PDU made by Key::pdu
pub fn set_mic(pdu: &mut [u8], mic: &[u8; 16]) {
    if pdu.len() >= 4 + MIC_OFF + MIC_LEN {
        pdu[4 + MIC_OFF..4 + MIC_OFF + MIC_LEN].copy_from_slice(mic);
    }
}

/// the RSN element of a personal network: CCMP for group and pairwise traffic, one AKM
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rsne {
    pub akm: u8,
    pub caps: u16,
}

impl Rsne {
    /// the element with its id and length (22 bytes)
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut v = alloc::vec![EID_RSN, 20, 1, 0];
        v.extend_from_slice(&OUI);
        v.push(CIPHER_CCMP);
        v.extend_from_slice(&[1, 0]);
        v.extend_from_slice(&OUI);
        v.push(CIPHER_CCMP);
        v.extend_from_slice(&[1, 0]);
        v.extend_from_slice(&OUI);
        v.push(self.akm);
        v.extend_from_slice(&self.caps.to_le_bytes());
        v
    }
    /// the first RSN element of a list of elements (key data); one pairwise and one AKM suite
    /// of 00-0F-AC are read, the counts are little endian as in every element
    pub fn parse(elems: &[u8]) -> Option<Rsne> {
        let e = elements(elems).into_iter().find(|e| e.0 == EID_RSN)?.1;
        if e.len() < 18 || e[0] != 1 || e[1] != 0 {
            return None;
        }
        let np = u16::from_le_bytes([e[6], e[7]]) as usize;
        let at = 8 + 4 * np;
        if e.len() < at + 6 {
            return None;
        }
        let akm = e[at + 5];
        let caps = if e.len() >= at + 8 { u16::from_le_bytes([e[at + 6], e[at + 7]]) } else { 0 };
        Some(Rsne { akm, caps })
    }
}

/// the elements of a list: (id, contents); stops at one that does not fit
pub fn elements(b: &[u8]) -> Vec<(u8, &[u8])> {
    let mut v = Vec::new();
    let mut i = 0;
    while i + 2 <= b.len() {
        let (id, len) = (b[i], b[i + 1] as usize);
        if i + 2 + len > b.len() {
            break;
        }
        v.push((id, &b[i + 2..i + 2 + len]));
        i += 2 + len;
    }
    v
}

/// the GTK KDE: key id (0..3), transmit bit clear, the key
pub fn kde_gtk(id: u8, gtk: &[u8]) -> Vec<u8> {
    let mut v = alloc::vec![EID_VENDOR, (6 + gtk.len()) as u8];
    v.extend_from_slice(&OUI);
    v.push(KDE_GTK);
    v.extend_from_slice(&[id & 3, 0]);
    v.extend_from_slice(gtk);
    v
}

/// the IGTK KDE (management frame protection): key id (4 or 5), its packet number, the key
pub fn kde_igtk(id: u16, ipn: u64, igtk: &[u8]) -> Vec<u8> {
    let mut v = alloc::vec![EID_VENDOR, (12 + igtk.len()) as u8];
    v.extend_from_slice(&OUI);
    v.push(KDE_IGTK);
    v.extend_from_slice(&id.to_le_bytes());
    v.extend_from_slice(&ipn.to_le_bytes()[..6]);
    v.extend_from_slice(igtk);
    v
}

/// the KDEs of key data: (type, contents after the type); other elements are left out
pub fn kdes(data: &[u8]) -> Vec<(u8, &[u8])> {
    elements(data).into_iter().filter(|e| e.0 == EID_VENDOR && e.1.len() >= 4 && e.1[..3] == OUI).map(|e| (e.1[3], &e.1[4..])).collect()
}

/// key data padded for the key wrap: 0xdd and zeros up to a multiple of 8, at least 16 bytes
pub fn pad_key_data(data: &[u8]) -> Vec<u8> {
    let mut v = data.to_vec();
    if v.len() % 8 != 0 || v.len() < 16 {
        v.push(EID_VENDOR);
        while v.len() % 8 != 0 || v.len() < 16 {
            v.push(0);
        }
    }
    v
}
