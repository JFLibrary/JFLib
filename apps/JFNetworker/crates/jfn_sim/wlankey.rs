// wlankey -- the keys of WPA2-Personal and WPA3-Personal and the EAPOL-Key frames of the 4-way
// handshake (IEEE 802.11-2020 12.7): real cryptography, so a frame of the recording is what a
// capture would show
//
//   pmk_psk(passphrase, ssid)          WPA2: PBKDF2-HMAC-SHA1, 4096 rounds, 32 bytes (J.4)
//   pmk_sae(password, a, b, ra, rb)    WPA3: a stand-in for the PMK of SAE (see below)
//   Ptk::derive(sae, pmk, aa, spa, anonce, snonce)   KCK, KEK and TK: WPA2 the PRF-384 of
//                                      12.7.1.2 over HMAC-SHA1, WPA3 (AKM 8) KDF-SHA256-384
//   m1 / m2 / m3 / m4                  the four messages as netpkt::eapol::Key
//   seal(key, ptk, sae, version)       the EAPOL PDU with its MIC (HMAC-SHA1-128 for WPA2, AES-128-
//                                      CMAC for AKM 8); mic_ok(pdu, ptk, sae) checks one
//   key_data(sae, gtk, igtk) / wrap / unwrap_gtk   message 3's key data: the RSN element, the
//                                      GTK (and for WPA3 the IGTK) KDE, padded, AES-wrapped
//   frame(dst, src, pdu)               the Ethernet II frame (ethertype 0x888E)
//
// SAE (the Dragonfly exchange of WPA3) is not computed: its commit and confirm are management
// frames, which are no frames here (jfn_sim::wlan models their time and outcome).  The PMK it
// would give both sides is stood in for by HKDF-SHA256 over the password, both MAC addresses and
// one random value of each side -- equal on both sides when the passwords are, fresh for every
// association, as SAE's is.  Everything after it (the handshake, its MICs, the key wrap) is the
// real thing.

use alloc::vec::Vec;

use netpkt::eapol::{self, Key, Rsne, KI_ACK, KI_ENCRYPTED, KI_INSTALL, KI_MIC, KI_PAIRWISE, KI_SECURE};
use netpkt::eth::{EthHdr, ET_EAPOL};
use netpkt::Mac;

/// the label of the pairwise key expansion
const LABEL: &[u8] = b"Pairwise key expansion";

/// the PMK of WPA2-Personal: PBKDF2-HMAC-SHA1(passphrase, SSID, 4096, 256 bits)
pub fn pmk_psk(passphrase: &str, ssid: &str) -> [u8; 32] {
    let mut out = [0u8; 32];
    crypto::pbkdf2_sha1(passphrase.as_bytes(), ssid.as_bytes(), 4096, &mut out);
    out
}

/// a stand-in for the PMK of SAE: HKDF-SHA256 over the password with both MAC addresses (the
/// smaller first) as salt and the random values of both sides as context
pub fn pmk_sae(password: &str, a: Mac, b: Mac, ra: &[u8; 32], rb: &[u8; 32]) -> [u8; 32] {
    let (lo, hi) = if a.0 <= b.0 { (a, b) } else { (b, a) };
    let mut salt = Vec::with_capacity(12);
    salt.extend_from_slice(&lo.0);
    salt.extend_from_slice(&hi.0);
    let (x, y) = if ra <= rb { (ra, rb) } else { (rb, ra) };
    let mut info = Vec::with_capacity(80);
    info.extend_from_slice(b"SAE PMK stand-in");
    info.extend_from_slice(x);
    info.extend_from_slice(y);
    let mut out = [0u8; 32];
    crypto::hkdf_sha256(&salt, password.as_bytes(), &info, &mut out);
    out
}

/// the pairwise keys
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ptk {
    /// the key of the MICs
    pub kck: [u8; 16],
    /// the key of the key wrap
    pub kek: [u8; 16],
    /// the temporal key (CCMP)
    pub tk: [u8; 16],
}

/// Min(AA, SPA) || Max(AA, SPA) || Min(ANonce, SNonce) || Max(ANonce, SNonce)
fn context(aa: Mac, spa: Mac, anonce: &[u8; 32], snonce: &[u8; 32]) -> Vec<u8> {
    let mut b = Vec::with_capacity(76);
    let (m0, m1) = if aa.0 <= spa.0 { (aa, spa) } else { (spa, aa) };
    b.extend_from_slice(&m0.0);
    b.extend_from_slice(&m1.0);
    let (n0, n1) = if anonce <= snonce { (anonce, snonce) } else { (snonce, anonce) };
    b.extend_from_slice(n0);
    b.extend_from_slice(n1);
    b
}

/// PRF-n of 12.7.1.2: HMAC-SHA1(K, A || 0 || B || i) for i = 0, 1, ... (n bits)
pub fn prf_sha1(key: &[u8], label: &[u8], data: &[u8], bits: usize) -> Vec<u8> {
    let n = bits / 8;
    let mut out = Vec::with_capacity(n + 20);
    let mut i = 0u8;
    while out.len() < n {
        let mut m = Vec::with_capacity(label.len() + data.len() + 2);
        m.extend_from_slice(label);
        m.push(0);
        m.extend_from_slice(data);
        m.push(i);
        out.extend_from_slice(&crypto::hmac_sha1(key, &m));
        i = i.wrapping_add(1);
    }
    out.truncate(n);
    out
}

/// KDF-SHA256-n of 12.7.1.6.2: HMAC-SHA256(K, i || label || context || n) with 16-bit
/// little-endian i (from 1) and n (bits)
pub fn kdf_sha256(key: &[u8], label: &[u8], context: &[u8], bits: usize) -> Vec<u8> {
    let n = bits / 8;
    let mut out = Vec::with_capacity(n + 32);
    let mut i = 1u16;
    while out.len() < n {
        let mut m = Vec::with_capacity(label.len() + context.len() + 4);
        m.extend_from_slice(&i.to_le_bytes());
        m.extend_from_slice(label);
        m.extend_from_slice(context);
        m.extend_from_slice(&(bits as u16).to_le_bytes());
        out.extend_from_slice(&crypto::hmac_sha256(key, &m));
        i += 1;
    }
    out.truncate(n);
    out
}

impl Ptk {
    /// the keys of a handshake: WPA2 (AKM 2) the PRF-384, WPA3 (AKM 8) KDF-SHA256-384
    pub fn derive(sae: bool, pmk: &[u8; 32], aa: Mac, spa: Mac, anonce: &[u8; 32], snonce: &[u8; 32]) -> Ptk {
        let ctx = context(aa, spa, anonce, snonce);
        let b = if sae { kdf_sha256(pmk, LABEL, &ctx, 384) } else { prf_sha1(pmk, LABEL, &ctx, 384) };
        let mut p = Ptk { kck: [0; 16], kek: [0; 16], tk: [0; 16] };
        p.kck.copy_from_slice(&b[..16]);
        p.kek.copy_from_slice(&b[16..32]);
        p.tk.copy_from_slice(&b[32..48]);
        p
    }
}

/// the key descriptor version: 2 (HMAC-SHA1-128, AES key wrap) for WPA2, 0 (defined by AKM 8)
/// for WPA3
fn kdv(sae: bool) -> u16 {
    if sae { eapol::KDV_AKM } else { eapol::KDV_SHA1 }
}

/// the RSN element of a network (CCMP, PSK or SAE; WPA3 requires management frame protection)
pub fn rsne(sae: bool) -> Rsne {
    if sae { Rsne { akm: eapol::AKM_SAE, caps: eapol::RSN_MFPR | eapol::RSN_MFPC } } else { Rsne { akm: eapol::AKM_PSK, caps: 0 } }
}

/// message 1: the AP's nonce
pub fn m1(sae: bool, replay: u64, anonce: &[u8; 32]) -> Key {
    Key { info: kdv(sae) | KI_PAIRWISE | KI_ACK, key_len: 16, replay, nonce: *anonce, ..Key::default() }
}

/// message 2: the station's nonce and its RSN element, with a MIC
pub fn m2(sae: bool, replay: u64, snonce: &[u8; 32]) -> Key {
    Key { info: kdv(sae) | KI_PAIRWISE | KI_MIC, key_len: 0, replay, nonce: *snonce, data: rsne(sae).to_bytes(), ..Key::default() }
}

/// message 3: install, the AP's nonce again, the wrapped key data
pub fn m3(sae: bool, replay: u64, anonce: &[u8; 32], wrapped: Vec<u8>) -> Key {
    Key { info: kdv(sae) | KI_PAIRWISE | KI_INSTALL | KI_ACK | KI_MIC | KI_SECURE | KI_ENCRYPTED, key_len: 16, replay, nonce: *anonce, data: wrapped, ..Key::default() }
}

/// message 4: the acknowledgement
pub fn m4(sae: bool, replay: u64) -> Key {
    Key { info: kdv(sae) | KI_PAIRWISE | KI_MIC | KI_SECURE, key_len: 0, replay, ..Key::default() }
}

/// the MIC of a PDU with its MIC field zero
fn mic(sae: bool, kck: &[u8; 16], zeroed: &[u8]) -> [u8; 16] {
    if sae {
        crypto::cmac_aes(kck, zeroed).unwrap_or([0; 16])
    } else {
        let h = crypto::hmac_sha1(kck, zeroed);
        let mut m = [0u8; 16];
        m.copy_from_slice(&h[..16]);
        m
    }
}

/// the EAPOL PDU of a key message; with the MIC bit, its MIC under the KCK
pub fn seal(k: &Key, ptk: Option<&Ptk>, sae: bool, version: u8) -> Vec<u8> {
    let mut pdu = k.pdu(version);
    if k.has(KI_MIC)
        && let Some(p) = ptk
        && let Some(z) = eapol::mic_input(&pdu)
    {
        let m = mic(sae, &p.kck, &z);
        eapol::set_mic(&mut pdu, &m);
    }
    pdu
}

/// the MIC of a received PDU is right under these keys
pub fn mic_ok(pdu: &[u8], ptk: &Ptk, sae: bool) -> bool {
    let Some(z) = eapol::mic_input(pdu) else { return false };
    let Some(k) = eapol::EapolView::parse(pdu).and_then(|v| Key::parse(v.body())) else { return false };
    crypto::ct_eq(&mic(sae, &ptk.kck, &z), &k.mic)
}

/// message 3's key data before the wrap: the AP's RSN element, the GTK (key 1) and, with
/// management frame protection (WPA3), the IGTK (key 4), padded
pub fn key_data(sae: bool, gtk: &[u8; 16], igtk: &[u8; 16]) -> Vec<u8> {
    let mut d = rsne(sae).to_bytes();
    d.extend_from_slice(&eapol::kde_gtk(1, gtk));
    if sae {
        d.extend_from_slice(&eapol::kde_igtk(4, 0, igtk));
    }
    eapol::pad_key_data(&d)
}

/// the key data wrapped under the KEK
pub fn wrap(kek: &[u8; 16], data: &[u8]) -> Vec<u8> {
    crypto::aes_wrap(kek, data).unwrap_or_default()
}

/// the GTK out of message 3's wrapped key data (None: it does not unwrap)
pub fn unwrap_gtk(kek: &[u8; 16], data: &[u8]) -> Option<[u8; 16]> {
    let plain = crypto::aes_unwrap(kek, data)?;
    let (_, g) = eapol::kdes(&plain).into_iter().find(|k| k.0 == eapol::KDE_GTK)?;
    let mut gtk = [0u8; 16];
    gtk.copy_from_slice(g.get(2..18)?);
    Some(gtk)
}

/// the Ethernet II frame of an EAPOL PDU (padded to 60 bytes)
pub fn frame(dst: Mac, src: Mac, pdu: &[u8]) -> Vec<u8> {
    let mut f = EthHdr { dst, src, ethertype: ET_EAPOL }.frame(pdu);
    netpkt::eth::pad(&mut f);
    f
}
