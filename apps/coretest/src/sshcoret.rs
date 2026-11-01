// sshcoret -- checks of netpkt::ssh (group sshcore; the group ssh is netio's blocking client): the
// wire types (the mpints of RFC 4251 5), KEXINIT and its negotiation, the exchange hash and the key
// derivation of curve25519-sha256 against vectors made outside this library (hashlib; three shared
// secrets for the mpint's cases), one binary packet per cipher against vectors made with a small
// ChaCha20 / Poly1305 and openssl (AES-CTR with HMAC), a changed byte refused; keys: ssh-keygen's
// file read, public lines, fingerprints, RSA generated, signed, verified and back from its file;
// a client and a server core talking in memory: every cipher, both host key kinds, bytes in pieces,
// host key refused, password, key, none, wrong signature, exec, shell, exit status, windows with
// megabytes, a new key exchange in the middle, a disconnect, the same seeds giving the same bytes;
// dissect and decode::layers of what they send; the core against netio::ssh's test server and
// netio's client against the core over TCP.  --ssh-core-serve PORT is the test server of
// tools/ssh_interop.py --core, where OpenSSH's own client logs in

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;

use netpkt::decode;
use netpkt::eth::{EthHdr, ET_IPV4};
use netpkt::ipv4::{Ipv4Hdr, PROTO_TCP};
use netpkt::pkt::{Pkt, HEADROOM};
use netpkt::ssh::{self, AuthMethod, Cipher, Config, Core, Event, KexInit, KeyPair, Request, R, W};
use netpkt::tcp::{TcpHdr, TcpOpts, ACK, PSH};
use netpkt::{Ipv4, Mac};
use plat::net::{IpAddr, SocketAddr, TcpListener, TcpStream};

use crate::T;

fn h(s: &str) -> Vec<u8> {
    crypto::unhex(&s.replace(' ', "")).expect("hex")
}

fn seed(n: u8) -> [u8; 32] {
    [n; 32]
}

fn host_ed() -> KeyPair {
    KeyPair::ed25519(&crypto::sha256(b"sshcore host key"))
}
fn user_ed() -> KeyPair {
    KeyPair::ed25519(&crypto::sha256(b"sshcore user key"))
}
/// a key's random numbers: the same every run
fn rand_from(tag: &'static [u8]) -> impl FnMut(&mut [u8]) {
    let mut n = 0u64;
    move |out: &mut [u8]| {
        for part in out.chunks_mut(32) {
            let mut x = Vec::from(tag);
            x.extend_from_slice(&n.to_be_bytes());
            part.copy_from_slice(&crypto::sha256(&x)[..part.len()]);
            n += 1;
        }
    }
}

fn wire(t: &mut T) {
    // RFC 4251 5
    t.eq("mpint 0", W::default().mpint(&[]).0, h("00000000"));
    t.eq("mpint 0 from zeros", W::default().mpint(&[0, 0]).0, h("00000000"));
    t.eq("mpint 9a378f9b2e332a7", W::default().mpint(&h("09a378f9b2e332a7")).0, h("00000008 09a378f9b2e332a7"));
    t.eq("mpint 80", W::default().mpint(&[0x80]).0, h("00000002 0080"));
    t.eq("string testing", W::default().str("testing").0, h("00000007 74657374696e67"));
    let mut r = R::new(&[0, 0, 0, 9, b'z', b'l', b'i', b'b', b',', b'n', b'o', b'n', b'e', 0, 0, 0, 200, 1]);
    t.eq("name-list", r.names(), Some(alloc::vec![String::from("zlib"), String::from("none")]));
    t.eq("string past the end", r.bytes().is_none(), true);
    t.eq("names of messages", (ssh::msg_name(20), ssh::msg_name(94), ssh::msg_name(200)), ("SSH_MSG_KEXINIT", "SSH_MSG_CHANNEL_DATA", "unknown message"));
    t.eq("reasons", (ssh::reason_name(2), ssh::reason_name(11)), ("protocol error", "by application"));
}

fn names(l: &[&str]) -> Vec<String> {
    l.iter().map(|s| String::from(*s)).collect()
}

fn kexinit(t: &mut T) {
    let k = KexInit {
        cookie: [7; 16],
        kex: names(&["curve25519-sha256", "curve25519-sha256@libssh.org"]),
        host_key: names(&["ssh-ed25519", "rsa-sha2-256"]),
        enc_cs: names(&["chacha20-poly1305@openssh.com", "aes128-ctr"]),
        enc_sc: names(&["aes128-ctr", "chacha20-poly1305@openssh.com"]),
        mac_cs: names(&["hmac-sha2-256"]),
        mac_sc: names(&["hmac-sha2-256"]),
        comp_cs: names(&["none"]),
        comp_sc: names(&["none"]),
        lang_cs: Vec::new(),
        lang_sc: Vec::new(),
        follows: false,
    };
    let p = k.to_payload();
    t.eq("KEXINIT back", KexInit::parse(&p), Some(k.clone()));
    t.eq("KEXINIT begins", (p[0], p[1], p[17..21].to_vec()), (20, 7, alloc::vec![0, 0, 0, 46]));
    t.check("KEXINIT cut off", KexInit::parse(&p[..p.len() - 6]).is_none());
    let mut s = k.clone();
    s.enc_cs = names(&["aes256-ctr", "aes128-ctr", "chacha20-poly1305@openssh.com"]);
    s.host_key = names(&["rsa-sha2-256"]);
    let a = ssh::negotiate(&k, &s).expect("common");
    t.eq("the client's order decides", (a.enc_cs.as_str(), a.host_key.as_str(), a.kex.as_str()), ("chacha20-poly1305@openssh.com", "rsa-sha2-256", "curve25519-sha256"));
    t.eq("an AEAD needs no MAC", (a.mac_cs.as_str(), a.mac_sc.as_str()), ("", "hmac-sha2-256"));
    let mut none = k.clone();
    none.host_key = names(&["ecdsa-sha2-nistp256"]);
    t.check("nothing in common", ssh::negotiate(&k, &none).is_none());
}

/// RFC 7748 6.1's public keys; hashlib's answers (build notes: gen_ssh_vectors in the report)
fn exchange(t: &mut T) {
    let (qc, qs) = (h("8520f0098930a754748b7ddcb43ef75a0dbf3a0d26381af4eba4a98eaa9b4e6a"), h("de9edb7d7b7dc1b4d35b61c2ece435373f8343c85b78674dadfc7e146f882b4f"));
    let mut ic = Vec::from(&[0x14u8][..]);
    ic.extend(0..16u8);
    ic.extend_from_slice(b"client-kexinit");
    let mut is = Vec::from(&[0x14u8][..]);
    is.extend(16..32u8);
    is.extend_from_slice(b"server-kexinit");
    let ks = W::default().str("ssh-ed25519").bytes(&(0..32u8).collect::<Vec<u8>>()).0;
    let cases = [
        ("K", "4a5d9d5ba4ce2de1728e3bf480350f25e07e21c947d19e3376f09b3c1e161742", "52cfa820b273ca426b8aa8bfd56c4e65b0d59af3ea5b9a92840b7b7021c749c6", "871959419b34084b3e9d277d046cbb08", "18f8620ac89e0fdc315c82b71929d9c4d725a50d04cfa91e8ffb5f811c94b2f2307d41504bda7873d9c20186403163a6df50ede1cb0e1325e5f6d03ba0f17cfb", "b6ecf761bae3b5d6ec724bd05e91fb20dc045ac586f926d93ce536367aa8cad5", "02a8a7814eceaf83fccb75fbb7bae1f1f86dd41e77605922550411b54d61200e947e88c85ba22574c3b4b64bf01be436fa734fa98f2affb856764528feb72eb6709c53c24dec7dd7649498ee32ba2e617ff1d801a8436acf6b8a5788410b428c"),
        ("top bit", "ca5d9d5ba4ce2de1728e3bf480350f25e07e21c947d19e3376f09b3c1e161742", "08d166c2a8173ad3671d9cc869bff08525d40c6f3f4e3f152c6b34ce2c34f9a1", "839c5d9de3a939a98d607b54da57a1bb", "2080f34f7fa6835e17d121ed53bded7d9362c7bce2b8249681e0eb3f9cb7679498442dc05873e9a9202fdebe35b4ea3f94d499e2be06cd6c756167fd2b404150", "fdf81b2bdf53e47e415fb4eee569c5d3f464dd6cccdc12156a49eca03e0cc9a9", "955141ae03df7bb0223c5d7143f5a9acc1bcb233e7197056334cec9027b76b9e8f647f7951ae5174ffd3fd8af9af3dbf45fcd9564ffcd2893afb80791feb85fd180405f0fd0c60b8077a1b524f6cae9caf566a44a0b5845e3a7f12aa4d20b6a3"),
        ("leading zeros", "00005d9d5ba4ce2de1728e3bf480350f25e07e21c947d19e3376f09b3c1e1617", "9bc2369d25c102d3b311cc39c3e0ceda15daac9f17e1e2752e551df55d6008c4", "76205697843f70b36cedd17c697f953b", "490660c0a19bbd7cac34c242bbb77c8dc5c361ec8779aaadb3ba98d5b728d3c5f9e354dd17bd2166134639b09ca692b41de7095ac6afb37fee33d8dbc88dbdea", "f96fa782a157cc446b9743a5ad10dd8ddb32f7fe621413577c88513ec0d7fcca", "4f68b29a4e776fe6acc64297d5073249ce24a89e2041e9591376588a1981e9b9263cb3ee4b5bb9c85f9cbca85e39d5259bdb2406b04a5093ca4814e5f3657f7dec8df460c0cb02fd3670c21cafa53dbd2600d11ea574bb1ca8ce775709904353"),
    ];
    for (name, k, hh, a, c, e, f) in cases {
        let k = h(k);
        let x = ssh::exchange_hash("SSH-2.0-OpenSSH_9.9", "SSH-2.0-JFNetworker_1", &ic, &is, &ks, &qc, &qs, &k);
        t.eq(&format!("exchange hash, {}", name), x.to_vec(), h(hh));
        t.eq(&format!("key A, {}", name), ssh::derive_key(&k, &x, b'A', &x, 16), h(a));
        t.eq(&format!("key C (two rounds), {}", name), ssh::derive_key(&k, &x, b'C', &x, 64), h(c));
        t.eq(&format!("key E, {}", name), ssh::derive_key(&k, &x, b'E', &x, 32), h(e));
        t.eq(&format!("key F (three rounds), {}", name), ssh::derive_key(&k, &x, b'F', &x, 96), h(f));
    }
    // RFC 7748 6.1 itself: both sides reach that shared secret
    let a = h("77076d0a7318a57d3c16c17251b26645df4c2f87ebc0992ab177fba51db92c2a");
    let mut sk = [0u8; 32];
    sk.copy_from_slice(&a);
    let mut q = [0u8; 32];
    q.copy_from_slice(&qs);
    t.eq("the shared secret", crypto::x25519::shared(&sk, &q).map(|s| s.to_vec()), Some(h(cases[0].1)));
}

fn packets(t: &mut T) {
    let payload = h("5e 00000000 00000005 68656c6c6f");
    let pad: Vec<u8> = (0xA0..0xC0).collect();
    let key: Vec<u8> = (0..64).collect();
    let cases: [(&str, &[u8], &str, usize); 3] = [
        ("chacha20-poly1305@openssh.com", &key, "a39afcb2211815434e832a5e6c68d395bbe3bc8c95109392db459a2bdda2f950df585b26489c2e8e83d9eb45", 9),
        ("aes128-ctr", &key[..16], "07feef68ec8b036e900eee118e91faf6feaeaaa479ae21a6e3dfdaca225375a4431d3bebdfcbee37b8286842b3d31038766b505edd8fda6febcbe3446c30c2bf", 13),
        ("aes256-ctr", &key[..32], "e9c3ef96bf6a53e6f0749cd636e2c0eb95ad243a5a366745135437ebf4819b7f431d3bebdfcbee37b8286842b3d31038766b505edd8fda6febcbe3446c30c2bf", 13),
    ];
    let iv: Vec<u8> = (0x10..0x20).collect();
    let mac: Vec<u8> = (0x20..0x40).collect();
    for (name, k, want, npad) in cases {
        let mut c = Cipher::make(name, k, &iv, &mac);
        t.eq(&format!("{} padding", name), c.padding(payload.len()), npad);
        let w = c.seal(7, &payload, &pad[..npad]);
        t.eq(&format!("{} on the wire", name), w.clone(), h(want));
        // back: at once, in pieces, a changed byte, another sequence number
        let mut d = Cipher::make(name, k, &iv, &mac);
        let mut rx = w.clone();
        let mut head = None;
        t.eq(&format!("{} back", name), d.open(7, &mut rx, &mut head), Ok(Some(payload.clone())));
        t.check(&format!("{} takes its bytes", name), rx.is_empty());
        let mut d = Cipher::make(name, k, &iv, &mac);
        let (mut rx, mut head, mut got) = (Vec::new(), None, None);
        for b in &w {
            rx.push(*b);
            if let Ok(Some(p)) = d.open(7, &mut rx, &mut head) {
                got = Some(p);
            }
        }
        t.eq(&format!("{} byte by byte", name), got, Some(payload.clone()));
        let mut bad = w.clone();
        let n = bad.len();
        bad[n / 2] ^= 1;
        let mut d = Cipher::make(name, k, &iv, &mac);
        t.check(&format!("{} a changed byte", name), d.open(7, &mut bad, &mut None).is_err());
        let mut d = Cipher::make(name, k, &iv, &mac);
        t.check(&format!("{} another sequence number", name), d.open(8, &mut w.clone(), &mut None).is_err());
    }
    // no cipher yet: the sizes of RFC 4253 6
    let mut c = Cipher::None;
    let w = c.seal(0, &[21], &pad[..c.padding(1)]);
    t.eq("NEWKEYS in the clear", (w.len(), w[0..5].to_vec(), w[5]), (16, alloc::vec![0, 0, 0, 12, 10], 21));
    let (mut rx, mut head) = (w, None);
    t.eq("NEWKEYS back", Cipher::None.open(0, &mut rx, &mut head), Ok(Some(alloc::vec![21])));
    let mut big = Vec::from(&(0x0100_0000u32).to_be_bytes()[..]);
    big.extend_from_slice(&[0; 12]);
    t.check("a packet far too long", Cipher::None.open(0, &mut big, &mut None).is_err());
}

/// made by ssh-keygen -t ed25519 (sshd.rs has the same test vector)
const KEY: &str = concat!(
    "-----BEGIN OPENSSH PRIVATE KEY-----\n",
    "b3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQAAAAAAAAABAAAAMwAAAAtzc2gtZW\n",
    "QyNTUxOQAAACD60RnCg8OZDvdZHgvLvR7l2yKg+01chvq+oT2hdvRwLwAAAJDX5tuO1+bb\n",
    "jgAAAAtzc2gtZWQyNTUxOQAAACD60RnCg8OZDvdZHgvLvR7l2yKg+01chvq+oT2hdvRwLw\n",
    "AAAECSjwexJOO9WJ/WNjkFXHTz2Mv6RDbEsp8c43ZxXauvKPrRGcKDw5kO91keC8u9HuXb\n",
    "IqD7TVyG+r6hPaF29HAvAAAACGNvcmV0ZXN0AQIDBAU=\n",
    "-----END OPENSSH PRIVATE KEY-----\n",
);
const KEY_PUB: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIPrRGcKDw5kO91keC8u9HuXbIqD7TVyG+r6hPaF29HAv coretest";
const KEY_FP: &str = "SHA256:qH4NJXwAF64ckBxSLD7cVnVAUSfBFgrxuoJZ/uH8z+o";

fn keys(t: &mut T) {
    let (k, comment) = KeyPair::parse_private(KEY).expect("ssh-keygen's key");
    t.eq("comment of a key file", comment, String::from("coretest"));
    t.eq("public line", k.public_line("coretest"), String::from(KEY_PUB));
    t.eq("fingerprint", ssh::fingerprint(&k.blob()), String::from(KEY_FP));
    // byte for byte but the two check numbers, which ssh-keygen draws at random
    let body = |s: &str| crypto::b64::decode(&s.lines().filter(|l| !l.starts_with("-----")).collect::<String>()).unwrap_or_default();
    let (ours, theirs) = (body(&k.private_file("coretest")), body(KEY));
    let at = theirs.len() - 0x90;
    t.eq("our file is ssh-keygen's", (ours.len(), ours[..at].to_vec(), ours[at + 8..].to_vec()), (theirs.len(), theirs[..at].to_vec(), theirs[at + 8..].to_vec()));
    t.eq("its lines are as long", k.private_file("coretest").lines().nth(1).map(str::len), Some(70));
    let p = ssh::parse_public(KEY_PUB).expect("line");
    t.eq("parse_public", (p.key_type.as_str(), p.comment.as_str(), p.blob == k.blob()), ("ssh-ed25519", "coretest", true));
    t.check("a line of the wrong type", ssh::parse_public("ssh-rsa AAAAC3NzaC1lZDI1NTE5AAAAIPrRGcKDw5kO91keC8u9HuXbIqD7TVyG+r6hPaF29HAv").is_none());
    t.eq("kinds and bits", (k.key_type(), k.alg(), k.bits(), ssh::blob_type(&k.blob()), ssh::blob_bits(&k.blob())), ("ssh-ed25519", "ssh-ed25519", 256, "ssh-ed25519", 256));
    t.check("broken key file", KeyPair::parse_private(&KEY.replace("IqD7TVyG", "IqD7TVyH")).is_none());
    let sig = k.sign(b"data");
    t.check("ed25519 signature", ssh::verify("ssh-ed25519", &k.blob(), b"data", &sig));
    t.check("ed25519 other data", !ssh::verify("ssh-ed25519", &k.blob(), b"datb", &sig));
    t.check("ed25519 other algorithm", !ssh::verify("rsa-sha2-256", &k.blob(), b"data", &sig));
    // RSA: the same random bytes give the same key; it signs and comes back from its file
    let mut r = rand_from(b"sshcore rsa");
    let rk = KeyPair::rsa(1024, &mut r).expect("rsa");
    let mut r2 = rand_from(b"sshcore rsa");
    let rk2 = KeyPair::rsa(1024, &mut r2).expect("rsa");
    t.eq("the same bytes, the same key", rk.blob(), rk2.blob());
    t.eq("RSA kinds and bits", (rk.key_type(), rk.alg(), rk.bits(), ssh::blob_bits(&rk.blob())), ("ssh-rsa", "rsa-sha2-256", 1024, 1024));
    let sig = rk.sign(b"data");
    t.check("RSA signature", ssh::verify("rsa-sha2-256", &rk.blob(), b"data", &sig));
    t.check("RSA other data", !ssh::verify("rsa-sha2-256", &rk.blob(), b"datb", &sig));
    let (back, c) = KeyPair::parse_private(&rk.private_file("r1")).expect("RSA file");
    t.eq("RSA file", (back.blob() == rk.blob(), c.as_str()), (true, "r1"));
    t.check("RSA from the file signs", ssh::verify("rsa-sha2-256", &rk.blob(), b"x", &back.sign(b"x")));
    if let KeyPair::Rsa(p) = &rk {
        let (a, b, i) = (crypto::bn::from_be(p.p(), 0), crypto::bn::from_be(p.q(), 0), crypto::bn::from_be(&p.iqmp(), 0));
        let m = crypto::bn::Mont::new(&a).expect("odd");
        // iqmp * q == 1 mod p
        let v = m.from_mont(&m.mul(&m.to_mont(&i), &m.to_mont(&b)));
        t.eq("iqmp", crypto::bn::bits(&v), 1);
        t.check("a key whose parts do not fit", crypto::rsa::PrivateKey::from_parts(&p.n(), p.e(), p.e(), p.p(), p.q()).is_none());
    }
    let mut r3 = rand_from(b"sshcore rsa 2");
    t.check("too short a key", crypto::rsa::PrivateKey::generate(512, &mut r3).is_none());
}

/// moves what each side has to the other until both are quiet; the events in order
fn pump(c: &mut Core, s: &mut Core, ce: &mut Vec<Event>, se: &mut Vec<Event>) {
    loop {
        let a = c.take_out();
        let b = s.take_out();
        if a.is_empty() && b.is_empty() {
            break;
        }
        s.push(&a);
        c.push(&b);
        ce.extend(c.take_events());
        se.extend(s.take_events());
    }
    ce.extend(c.take_events());
    se.extend(s.take_events());
}

fn cfg(ciphers: &[&str]) -> Config {
    Config { ident: String::from("SSH-2.0-JFNetworker_1"), ciphers: names(ciphers), ..Config::default() }
}

/// a client and a server through the key exchange (the client takes the key)
fn connected(ciphers: &[&str], key: KeyPair) -> (Core, Core) {
    let mut c = Core::client(cfg(ciphers), &seed(1));
    let mut s = Core::server(cfg(ciphers), alloc::vec![key], &seed(2));
    let (mut ce, mut se) = (Vec::new(), Vec::new());
    pump(&mut c, &mut s, &mut ce, &mut se);
    c.host_key_verdict(true);
    pump(&mut c, &mut s, &mut ce, &mut se);
    (c, s)
}

/// logged in by password as the server takes it
fn logged_in(ciphers: &[&str], key: KeyPair) -> (Core, Core) {
    let (mut c, mut s) = connected(ciphers, key);
    let (mut ce, mut se) = (Vec::new(), Vec::new());
    c.auth_password("admin", "x");
    pump(&mut c, &mut s, &mut ce, &mut se);
    s.auth_accept();
    pump(&mut c, &mut s, &mut ce, &mut se);
    (c, s)
}

fn memory(t: &mut T) {
    // the exchange, event by event, with the key's blob and the verdict
    let (mut c, mut s) = (Core::client(cfg(&["chacha20-poly1305@openssh.com"]), &seed(1)), Core::server(cfg(&["chacha20-poly1305@openssh.com"]), alloc::vec![host_ed()], &seed(2)));
    let (mut ce, mut se) = (Vec::new(), Vec::new());
    pump(&mut c, &mut s, &mut ce, &mut se);
    t.eq("the client's first events", ce.len(), 3);
    t.check("it hears the server's line", matches!(&ce[0], Event::PeerIdent(l) if l == "SSH-2.0-JFNetworker_1"));
    t.check("the algorithms", matches!(&ce[1], Event::Algos(a) if a.enc_cs == "chacha20-poly1305@openssh.com" && a.kex == "curve25519-sha256" && a.host_key == "ssh-ed25519"));
    t.check("the host key waits", matches!(&ce[2], Event::HostKey { alg, blob } if alg == "ssh-ed25519" && *blob == host_ed().blob()));
    t.check("not encrypted yet", !c.encrypted());
    ce.clear();
    c.host_key_verdict(true);
    pump(&mut c, &mut s, &mut ce, &mut se);
    t.check("both encrypted", c.encrypted() && s.encrypted());
    t.eq("the same session", c.session_id(), s.session_id());
    t.eq("session id length", c.session_id().len(), 32);
    t.eq("peers", (c.peer_ident(), s.peer_ident()), ("SSH-2.0-JFNetworker_1", "SSH-2.0-JFNetworker_1"));
    t.check("ready to log in", ce.contains(&Event::Encrypted) && ce.contains(&Event::AuthReady));

    // a key the client refuses
    let (mut c, mut s) = (Core::client(cfg(&["aes128-ctr"]), &seed(1)), Core::server(cfg(&["aes128-ctr"]), alloc::vec![host_ed()], &seed(2)));
    let (mut ce, mut se) = (Vec::new(), Vec::new());
    pump(&mut c, &mut s, &mut ce, &mut se);
    c.host_key_verdict(false);
    pump(&mut c, &mut s, &mut ce, &mut se);
    t.check("host key refused: the client is done", c.dead());
    t.check("the server hears why", se.iter().any(|e| matches!(e, Event::Disconnected { reason: ssh::DISC_HOST_KEY_NOT_VERIFIABLE, .. })));

    // nothing in common
    let (mut c, mut s) = (Core::client(cfg(&["aes128-ctr"]), &seed(1)), Core::server(cfg(&["aes256-ctr"]), alloc::vec![host_ed()], &seed(2)));
    let (mut ce, mut se) = (Vec::new(), Vec::new());
    pump(&mut c, &mut s, &mut ce, &mut se);
    t.check("no common cipher ends it", ce.iter().chain(se.iter()).any(|e| matches!(e, Event::Failed { reason: ssh::DISC_KEY_EXCHANGE_FAILED, .. })));

    // every cipher, both kinds of host key, bytes one by one
    let mut r = rand_from(b"sshcore rsa");
    let rk = KeyPair::rsa(1024, &mut r).expect("rsa");
    for cipher in ["chacha20-poly1305@openssh.com", "aes128-ctr", "aes256-ctr"] {
        for key in [host_ed(), rk.clone()] {
            let name = format!("{} {}", cipher, key.key_type());
            let (mut c, mut s) = (Core::client(cfg(&[cipher]), &seed(3)), Core::server(cfg(&[cipher]), alloc::vec![key.clone()], &seed(4)));
            let (mut ce, mut se) = (Vec::new(), Vec::new());
            loop {
                let a = c.take_out();
                let b = s.take_out();
                if a.is_empty() && b.is_empty() {
                    if ce.iter().any(|e| matches!(e, Event::HostKey { .. })) && !c.encrypted() {
                        c.host_key_verdict(true);
                        continue;
                    }
                    break;
                }
                for x in a {
                    s.push(&[x]);
                }
                for x in b {
                    c.push(&[x]);
                }
                ce.extend(c.take_events());
                se.extend(s.take_events());
            }
            t.check(&format!("{}: encrypted", name), c.encrypted() && s.encrypted());
            t.check(&format!("{}: agreed", name), c.algos().enc_cs == cipher && s.algos().enc_sc == cipher && c.algos().host_key == key.alg());
            t.check(&format!("{}: authenticates or no MAC", name), (cipher.starts_with("chacha") && c.algos().mac_cs.is_empty()) || (!cipher.starts_with("chacha") && c.algos().mac_cs == "hmac-sha2-256"));
            c.auth_none("admin");
            pump(&mut c, &mut s, &mut ce, &mut se);
            t.check(&format!("{}: a login reached the server", name), se.iter().any(|e| matches!(e, Event::AuthRequest { user, method: AuthMethod::None } if user == "admin")));
        }
    }

    // logins
    let (mut c, mut s) = connected(&["chacha20-poly1305@openssh.com"], host_ed());
    let (mut ce, mut se) = (Vec::new(), Vec::new());
    c.auth_none("admin");
    pump(&mut c, &mut s, &mut ce, &mut se);
    t.eq("none reaches the server", se.pop(), Some(Event::AuthRequest { user: String::from("admin"), method: AuthMethod::None }));
    s.auth_reject();
    pump(&mut c, &mut s, &mut ce, &mut se);
    t.eq("the methods come back", ce.pop(), Some(Event::AuthFailure { methods: names(&["publickey", "password"]), partial: false }));
    s.banner("Kein Zutritt");
    pump(&mut c, &mut s, &mut ce, &mut se);
    t.eq("a banner", ce.pop(), Some(Event::Banner(String::from("Kein Zutritt"))));
    c.auth_password("admin", "falsch");
    pump(&mut c, &mut s, &mut ce, &mut se);
    t.eq("password", se.pop(), Some(Event::AuthRequest { user: String::from("admin"), method: AuthMethod::Password(String::from("falsch")) }));
    s.auth_reject();
    pump(&mut c, &mut s, &mut ce, &mut se);
    t.check("still not in", !c.authenticated() && !s.authenticated());
    // a key: the question, then the signature
    let u = user_ed();
    ce.clear();
    se.clear();
    c.auth_key("admin", &u);
    pump(&mut c, &mut s, &mut ce, &mut se);
    t.eq("the question", se.pop(), Some(Event::AuthRequest { user: String::from("admin"), method: AuthMethod::PublicKey { alg: String::from("ssh-ed25519"), blob: u.blob(), signed: false, valid: false } }));
    s.auth_pk_ok("ssh-ed25519", &u.blob());
    pump(&mut c, &mut s, &mut ce, &mut se);
    t.eq("the signature is good", se.pop(), Some(Event::AuthRequest { user: String::from("admin"), method: AuthMethod::PublicKey { alg: String::from("ssh-ed25519"), blob: u.blob(), signed: true, valid: true } }));
    s.auth_accept();
    pump(&mut c, &mut s, &mut ce, &mut se);
    t.check("logged in", c.authenticated() && s.authenticated() && ce.contains(&Event::AuthSuccess));

    // the key of the login, with a host key as the user's
    let (mut c, mut s) = connected(&["aes128-ctr"], host_ed());
    let (mut ce, mut se) = (Vec::new(), Vec::new());
    c.auth_key("admin", &host_ed());
    pump(&mut c, &mut s, &mut ce, &mut se);
    s.auth_pk_ok("ssh-ed25519", &host_ed().blob());
    pump(&mut c, &mut s, &mut ce, &mut se);
    t.check("a key's own signature is good", se.iter().any(|e| matches!(e, Event::AuthRequest { method: AuthMethod::PublicKey { signed: true, valid: true, .. }, .. })));

    // channels: pty, shell, data, exit status, close
    let (mut c, mut s) = logged_in(&["chacha20-poly1305@openssh.com"], host_ed());
    let (mut ce, mut se) = (Vec::new(), Vec::new());
    let id = c.open_session();
    pump(&mut c, &mut s, &mut ce, &mut se);
    t.eq("the server is asked", se.pop(), Some(Event::ChannelOpen { channel: 0, kind: String::from("session") }));
    s.accept_channel(0);
    pump(&mut c, &mut s, &mut ce, &mut se);
    t.eq("the client hears", ce.pop(), Some(Event::ChannelOpened { channel: id }));
    c.request(id, &Request::Pty { term: String::from("xterm"), cols: 80, rows: 24 }, true);
    c.request(id, &Request::Shell, true);
    pump(&mut c, &mut s, &mut ce, &mut se);
    t.eq("pty", se.first().cloned(), Some(Event::ChannelRequest { channel: 0, request: Request::Pty { term: String::from("xterm"), cols: 80, rows: 24 }, want_reply: true }));
    t.eq("shell", se.get(1).cloned(), Some(Event::ChannelRequest { channel: 0, request: Request::Shell, want_reply: true }));
    s.reply(0, true);
    s.reply(0, false);
    pump(&mut c, &mut s, &mut ce, &mut se);
    t.eq("answers", (ce.first().cloned(), ce.get(1).cloned()), (Some(Event::ChannelReply { channel: id, ok: true }), Some(Event::ChannelReply { channel: id, ok: false })));
    ce.clear();
    se.clear();
    c.send(id, b"show version\n");
    s.send(0, b"R1#");
    s.send_ext(0, 1, b"warn");
    pump(&mut c, &mut s, &mut ce, &mut se);
    t.eq("data to the server", se.pop(), Some(Event::Data { channel: 0, ext: None, data: Vec::from(&b"show version\n"[..]) }));
    t.eq("data to the client", ce.first().cloned(), Some(Event::Data { channel: id, ext: None, data: Vec::from(&b"R1#"[..]) }));
    t.eq("stderr to the client", ce.get(1).cloned(), Some(Event::Data { channel: id, ext: Some(1), data: Vec::from(&b"warn"[..]) }));
    ce.clear();
    s.request(0, &Request::ExitStatus(3), false);
    s.eof(0);
    s.close(0);
    pump(&mut c, &mut s, &mut ce, &mut se);
    t.eq("exit status", ce.first().cloned(), Some(Event::ChannelRequest { channel: id, request: Request::ExitStatus(3), want_reply: false }));
    t.eq("eof", ce.get(1).cloned(), Some(Event::Eof { channel: id }));
    t.eq("closed", ce.get(2).cloned(), Some(Event::Closed { channel: id }));
    t.check("gone on both sides", !c.channel_open(id) && !s.channel_open(0));
    t.check("closed at the server too", se.contains(&Event::Closed { channel: 0 }));
    // a channel of another kind is refused; a second session works
    let id2 = c.open_session();
    pump(&mut c, &mut s, &mut ce, &mut se);
    s.refuse_channel(1, 1, "nicht hier");
    pump(&mut c, &mut s, &mut ce, &mut se);
    t.eq("refused", ce.pop(), Some(Event::ChannelRefused { channel: id2, reason: 1, text: String::from("nicht hier") }));

    // exec with 3 MiB through both windows, a new key exchange in the middle
    let (mut c, mut s) = logged_in(&["chacha20-poly1305@openssh.com"], host_ed());
    let (mut ce, mut se) = (Vec::new(), Vec::new());
    let id = c.open_session();
    pump(&mut c, &mut s, &mut ce, &mut se);
    s.accept_channel(0);
    pump(&mut c, &mut s, &mut ce, &mut se);
    c.request(id, &Request::Exec(String::from("sha256")), true);
    pump(&mut c, &mut s, &mut ce, &mut se);
    t.check("exec", se.iter().any(|e| matches!(e, Event::ChannelRequest { request: Request::Exec(x), .. } if x == "sha256")));
    let big: Vec<u8> = (0..(3u32 << 20) + 17).map(|i| (i.wrapping_mul(2654435761) >> 24) as u8).collect();
    c.send(id, &big);
    t.check("what the window does not take waits", c.channel_pending(id) == big.len() - (2 << 20));
    let sid = Vec::from(c.session_id());
    let mut got = Vec::new();
    ce.clear();
    se.clear();
    let mut rekeyed = false;
    s.rekey();
    for _ in 0..64 {
        pump(&mut c, &mut s, &mut ce, &mut se);
        for e in se.drain(..) {
            match e {
                Event::Data { data, .. } => got.extend_from_slice(&data),
                Event::Rekeyed => rekeyed = true,
                _ => {}
            }
        }
        if got.len() == big.len() {
            break;
        }
    }
    t.eq("3 MiB through the window", crypto::sha256(&got), crypto::sha256(&big));
    t.check("a new key exchange in the middle", rekeyed && ce.contains(&Event::Rekeyed));
    t.eq("the session stays", Vec::from(c.session_id()), sid);
    c.eof(id);
    c.close(id);
    s.close(0);
    pump(&mut c, &mut s, &mut ce, &mut se);
    t.check("the channel ends after it", !c.channel_open(id));

    // a changed byte on the wire, a disconnect
    let (mut c, mut s) = logged_in(&["aes128-ctr"], host_ed());
    let (mut ce, mut se) = (Vec::new(), Vec::new());
    let _ = c.open_session();
    let mut w = c.take_out();
    let n = w.len();
    w[n - 1] ^= 1;
    s.push(&w);
    se.extend(s.take_events());
    t.check("a changed byte ends the connection", s.dead() && se.iter().any(|e| matches!(e, Event::Failed { reason: ssh::DISC_MAC_ERROR, .. })));
    let (mut c, mut s) = logged_in(&["aes256-ctr"], host_ed());
    s.disconnect(ssh::DISC_BY_APPLICATION, "bye");
    pump(&mut c, &mut s, &mut ce, &mut se);
    t.check("a disconnect", c.dead() && s.dead() && ce.iter().any(|e| matches!(e, Event::Disconnected { reason: ssh::DISC_BY_APPLICATION, text } if text == "bye")));
    // not an SSH peer
    let mut c = Core::client(cfg(&["aes256-ctr"]), &seed(1));
    c.push(b"SSH-1.5-old\r\n");
    t.check("version 1 refused", c.dead() && c.take_events().iter().any(|e| matches!(e, Event::Failed { reason: ssh::DISC_VERSION_NOT_SUPPORTED, .. })));
    let mut c = Core::client(cfg(&["aes256-ctr"]), &seed(1));
    c.push(b"Hallo\r\nnoch eine Zeile\r\nSSH-1.99-both\r\n");
    t.check("lines before the identification, 1.99 is version 2", !c.dead() && c.peer_ident() == "SSH-1.99-both");

    // the same seeds: the same bytes
    let run = || {
        let (mut c, mut s) = (Core::client(cfg(&["chacha20-poly1305@openssh.com"]), &seed(9)), Core::server(cfg(&["chacha20-poly1305@openssh.com"]), alloc::vec![host_ed()], &seed(10)));
        let mut wire = Vec::new();
        for _ in 0..8 {
            let (a, b) = (c.take_out(), s.take_out());
            wire.extend_from_slice(&a);
            wire.extend_from_slice(&b);
            s.push(&a);
            c.push(&b);
            c.host_key_verdict(true);
        }
        wire
    };
    t.eq("deterministic", crypto::sha256(&run()), crypto::sha256(&run()));
}

/// a frame with `tcp` on port 22
fn frame(tcp: &[u8]) -> Vec<u8> {
    let (a, b) = (Ipv4([10, 0, 0, 1]), Ipv4([10, 0, 0, 2]));
    let mut p = Pkt::new(HEADROOM, tcp);
    TcpHdr { src_port: 49152, dst_port: ssh::PORT, seq: 1, ack: 1, flags: ACK | PSH, window: 8192, urgent: 0, opts: TcpOpts::default() }.push_v4(&mut p, a, b);
    Ipv4Hdr::new(a, b, PROTO_TCP, 64).push_onto(&mut p);
    EthHdr { dst: Mac([2, 0, 0, 0, 0, 1]), src: Mac([2, 0, 0, 0, 0, 2]), ethertype: ET_IPV4 }.push_onto(&mut p);
    p.into_vec()
}

fn field<'a>(l: &'a decode::Layer, name: &str) -> Option<&'a str> {
    l.fields.iter().find(|f| f.name == name).map(|f| f.value.as_str())
}

fn dissector(t: &mut T) {
    let mut c = Core::client(cfg(&["chacha20-poly1305@openssh.com"]), &seed(1));
    let w = c.take_out();
    let f = frame(&w);
    t.check("the mask", decode::proto_mask(&f) & decode::P_SSH != 0);
    let ls = decode::layers(&f);
    let l = ls.iter().find(|l| l.proto == decode::Proto::Ssh).expect("SSH layer");
    t.eq("identification field", field(l, "Identification"), Some("SSH-2.0-JFNetworker_1"));
    t.eq("identification summary", decode::info(&f).1, String::from("SSH-2.0-JFNetworker_1"));
    // a KEXINIT in the clear
    c.push(b"SSH-2.0-other\r\n");
    let w = c.take_out();
    t.eq("dissect finds it", ssh::dissect(&w).iter().filter(|p| matches!(p, ssh::Part::Packet { code: ssh::MSG_KEXINIT, .. })).count(), 1);
    let f = frame(&w);
    let ls = decode::layers(&f);
    let l = ls.iter().find(|l| l.proto == decode::Proto::Ssh).expect("SSH layer");
    t.eq("message name", field(l, "Message Code"), Some("SSH_MSG_KEXINIT (20)"));
    t.eq("its ciphers", field(l, "Ciphers client to server"), Some("chacha20-poly1305@openssh.com"));
    t.eq("its key exchange", field(l, "KEX Algorithms"), Some("curve25519-sha256,curve25519-sha256@libssh.org"));
    t.check("packet fields", field(l, "Packet Length").is_some() && field(l, "Padding Length").is_some() && field(l, "Padding").is_some());
    t.eq("KEXINIT summary", decode::info(&f).1, String::from("SSH_MSG_KEXINIT"));
    t.check("no problems", l.problems.is_empty());
    // what comes after NEWKEYS
    let (mut c, _s) = logged_in(&["chacha20-poly1305@openssh.com"], host_ed());
    let _ = c.take_out();
    let _ = c.open_session();
    let w = c.take_out();
    let f = frame(&w);
    let ls = decode::layers(&f);
    let l = ls.iter().find(|l| l.proto == decode::Proto::Ssh).expect("SSH layer");
    t.eq("encrypted", field(l, "Encrypted Packet"), Some("verschlüsselt"));
    t.eq("encrypted summary", decode::info(&f).1, format!("verschlüsselt ({} bytes)", w.len()));
}

// ---------------------------------------------------------------- over TCP

/// runs a core over a socket until it is dead or the socket ends
fn drive(s: &TcpStream, core: &mut Core, on: &mut dyn FnMut(&mut Core, Event)) -> bool {
    let mut buf = alloc::vec![0u8; 65536];
    loop {
        for e in core.take_events() {
            on(core, e);
        }
        let out = core.take_out();
        if !out.is_empty() && s.write_all(&out).is_err() {
            return false;
        }
        if core.dead() {
            return true;
        }
        match s.read(&mut buf) {
            Ok(0) | Err(_) => return false,
            Ok(n) => core.push(&buf[..n]),
        }
    }
}

/// the commands of the test server over the core (sshd.rs's); `logins` the keys it takes
pub fn serve_core(s: TcpStream, logins: &crate::sshd::Logins, ciphers: &[&str]) {
    let mut core = Core::server(cfg(ciphers), alloc::vec![crate::sshd::host_key_pair()], &crypto::sha256(b"sshcore server"));
    let mut stdin: Vec<u8> = Vec::new();
    let mut cmd = String::new();
    let mut shell = false;
    let mut on = |c: &mut Core, e: Event| match e {
        Event::AuthRequest { user, method } => match method {
            AuthMethod::Password(p) if user == crate::sshd::USER && p == crate::sshd::PASSWORD => c.auth_accept(),
            AuthMethod::PublicKey { alg, blob, signed, valid } if user == crate::sshd::USER && logins.keys.contains(&blob) => {
                if !signed {
                    c.auth_pk_ok(&alg, &blob);
                } else if valid {
                    c.auth_accept();
                } else {
                    c.auth_reject();
                }
            }
            _ => c.auth_reject(),
        },
        Event::ChannelOpen { channel, kind } => {
            if kind == "session" {
                c.accept_channel(channel);
            } else {
                c.refuse_channel(channel, 3, "unknown channel type");
            }
        }
        Event::ChannelRequest { channel, request, want_reply } => {
            let ok = match request {
                Request::Exec(x) => {
                    cmd = x;
                    true
                }
                Request::Shell => {
                    shell = true;
                    c.send(channel, b"core> ");
                    true
                }
                Request::Pty { .. } | Request::Env { .. } => true,
                _ => false,
            };
            if want_reply {
                c.reply(channel, ok);
            }
        }
        Event::Data { channel, data, .. } => {
            if shell {
                c.send(channel, &data);
            } else {
                stdin.extend_from_slice(&data);
            }
        }
        Event::Eof { channel } => {
            let (out, err, code) = crate::sshd::run_command(&cmd, &stdin);
            c.send(channel, &out);
            for part in err.chunks(1024) {
                c.send_ext(channel, 1, part);
            }
            c.request(channel, &Request::ExitStatus(code), false);
            c.eof(channel);
            c.close(channel);
        }
        _ => {}
    };
    drive(&s, &mut core, &mut on);
}

fn tcp(t: &mut T) {
    let logins = Arc::new(crate::sshd::Logins { keys: alloc::vec![user_ed().blob()] });
    // the core as a client against netio's server
    let port = crate::sshd::spawn(1, logins.clone());
    t.check("netio's test server", port != 0);
    let Ok(s) = TcpStream::connect(&SocketAddr::new(IpAddr::LOCALHOST_V4, port), 3000) else {
        t.check("connect", false);
        return;
    };
    s.set_read_timeout(Some(10_000));
    let mut core = Core::client(cfg(&["chacha20-poly1305@openssh.com"]), &seed(5));
    let (mut out, mut status, mut hk, mut chan) = (Vec::new(), None, None, 0u32);
    let u = user_ed();
    let mut on = |c: &mut Core, e: Event| match e {
        Event::HostKey { blob, .. } => {
            hk = Some(ssh::fingerprint(&blob));
            c.host_key_verdict(true);
        }
        Event::AuthReady => c.auth_key(crate::sshd::USER, &u),
        Event::AuthSuccess => chan = c.open_session(),
        Event::ChannelOpened { channel } => {
            c.request(channel, &Request::Exec(String::from("echo hallo netio")), true);
            c.eof(channel);
        }
        Event::Data { data, .. } => out.extend_from_slice(&data),
        Event::ChannelRequest { request: Request::ExitStatus(n), .. } => status = Some(n),
        Event::Closed { .. } => c.disconnect(ssh::DISC_BY_APPLICATION, "fertig"),
        _ => {}
    };
    let ended = drive(&s, &mut core, &mut on);
    t.check("the core logs in at netio's server and is done", ended);
    t.eq("it saw netio's key", hk, Some(ssh::fingerprint(&crate::sshd::host_key_pair().blob())));
    t.eq("its command ran", out, Vec::from(&b"hallo netio\n"[..]));
    t.eq("with its exit status", status, Some(0));
    let _ = chan;

    // netio's client against the core as a server
    let Ok(l) = TcpListener::bind(&SocketAddr::new(IpAddr::LOCALHOST_V4, 0)) else {
        t.check("listen", false);
        return;
    };
    let port = l.local_addr().map_or(0, |a| a.port);
    let lg = logins.clone();
    let _ = plat::thread_spawn(256 * 1024, Box::new(move || {
        for _ in 0..2 {
            let Ok(Some((s, _))) = l.accept_timeout(10_000) else { return };
            s.set_read_timeout(Some(10_000));
            serve_core(s, &lg, &["chacha20-poly1305@openssh.com"]);
        }
    }));
    match crate::sshd::connect(port, netio::ssh::HostKeyCheck::Any) {
        Ok(mut c) => {
            t.eq("netio's client sees the core's key", c.host_key().fingerprint.clone(), ssh::fingerprint(&crate::sshd::host_key_pair().blob()));
            t.eq("wrong password", c.auth_password(crate::sshd::USER, "falsch").err(), Some(netio::ssh::SshError::Auth));
            t.check("password", c.auth_password(crate::sshd::USER, crate::sshd::PASSWORD).is_ok());
            match c.exec("fail", b"", 1 << 16) {
                Ok(x) => t.eq("exit code and stderr", (x.status, x.stderr), (Some(3), Vec::from(&b"boom\n"[..]))),
                Err(e) => t.check(&format!("exec: {}", e), false),
            }
            c.close();
        }
        Err(e) => t.check(&format!("netio's client: {}", e), false),
    }
    match crate::sshd::connect(port, netio::ssh::HostKeyCheck::Any) {
        Ok(mut c) => {
            let k = crypto::ed25519::Keypair::from_seed(&crypto::sha256(b"sshcore user key"));
            t.check("netio's client logs in with a key", c.auth_key(crate::sshd::USER, &k).is_ok());
            let big: Vec<u8> = (0..(1u32 << 20) + 5).map(|i| (i >> 3) as u8).collect();
            match c.exec("sha256", &big, 1 << 16) {
                Ok(x) => t.eq("1 MiB through stdin", String::from_utf8_lossy(&x.stdout).into_owned(), format!("{} {}\n", crypto::hex(&crypto::sha256(&big)), big.len())),
                Err(e) => t.check(&format!("exec: {}", e), false),
            }
            c.close();
        }
        Err(e) => t.check(&format!("netio's client 2: {}", e), false),
    }
}

pub fn run(t: &mut T) {
    wire(t);
    kexinit(t);
    exchange(t);
    packets(t);
    keys(t);
    memory(t);
    dissector(t);
    tcp(t);
    crate::snmpv3t::run(t);
}

/// --ssh-core-serve PORT [--authorized FILE] [--hostkey-out FILE] [--cipher NAME]: the core as a
/// server for OpenSSH's client (tools/ssh_interop.py --core)
pub fn serve(port: u16) -> i32 {
    let mut keys = Vec::new();
    if let Some(f) = plat::arg_value("--authorized") {
        for line in String::from_utf8_lossy(&plat::read_file(&f).unwrap_or_default()).lines() {
            if let Some(b) = ssh::parse_public(line) {
                keys.push(b.blob);
            }
        }
    }
    if let Some(f) = plat::arg_value("--hostkey-out") {
        plat::write_file(&f, format!("{}\n", crate::sshd::host_key_pair().public_line("coretest-host")).as_bytes());
    }
    let logins = Arc::new(crate::sshd::Logins { keys });
    let Ok(l) = TcpListener::bind(&SocketAddr::new(IpAddr::LOCALHOST_V4, port)) else { return 1 };
    plat::write_stdout(b"ready\n");
    loop {
        let Ok(Some((s, _))) = l.accept_timeout(60_000) else { return 0 };
        let lg = logins.clone();
        let _ = plat::thread_spawn(256 * 1024, Box::new(move || {
            s.set_read_timeout(Some(30_000));
            serve_core(s, &lg, &["chacha20-poly1305@openssh.com", "aes128-ctr", "aes256-ctr"]);
        }));
    }
}
