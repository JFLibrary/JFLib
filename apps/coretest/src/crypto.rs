// crypto -- known-answer vectors of lib/crypto (FIPS 180-4 / 197, SP 800-38D, RFC 4231 / 5869 /
// 7693 / 7748 / 7914 / 8032 / 8439 / 9106, draft-irtf-cfrg-xchacha, the BLAKE3 test vectors;
// RSA and ECDSA signatures made by OpenSSL through Python's cryptography) and round trips
// wpa(): what WPA2 and WPA3 build on -- SHA-1 (FIPS 180-4), HMAC-SHA1 (RFC 2202), PBKDF2-HMAC-SHA1
// (RFC 6070, the PSKs of IEEE 802.11 J.4), the AES key wrap (RFC 3394) and AES-CMAC (RFC 4493)

use alloc::string::String;
use alloc::vec::Vec;

use crypto::{aead, argon2, b64, blake2b, blake3, chacha, ed25519, hex, poly1305, sealbox, unhex, x25519};

use crate::T;

fn h(s: &str) -> Vec<u8> {
    unhex(s).expect("hex")
}

fn a32(s: &str) -> [u8; 32] {
    let mut a = [0u8; 32];
    a.copy_from_slice(&h(s));
    a
}

const SUNSCREEN: &[u8] = b"Ladies and Gentlemen of the class of '99: If I could offer you only one tip for the future, sunscreen would be it.";

pub fn run(t: &mut T) {
    sha2(t);
    hmac(t);
    blake2(t);
    blake3_vectors(t);
    chacha_poly(t);
    curve(t);
    argon(t);
    misc(t);
    web_pki(t);
    wpa(t);
}

/// the inverse cipher: FIPS 197 C.1 / C.3 backwards, then random blocks and keys both ways
fn aes_decrypt(t: &mut T) {
    let mut b = [0u8; 16];
    b.copy_from_slice(&h("69c4e0d86a7b0430d8cdb78070b4c55a"));
    crypto::aes::Aes::new(&h("000102030405060708090a0b0c0d0e0f")).expect("key").decrypt(&mut b);
    t.eq("aes-128 decrypt fips197", hex(&b), String::from("00112233445566778899aabbccddeeff"));
    b.copy_from_slice(&h("8ea2b7ca516745bfeafc49904b496089"));
    crypto::aes::Aes::new(&(0..32u8).collect::<Vec<u8>>()).expect("key").decrypt(&mut b);
    t.eq("aes-256 decrypt fips197", hex(&b), String::from("00112233445566778899aabbccddeeff"));
    let mut g = 0x5EC2E7u64;
    let mut next = || {
        g = g.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (g >> 33) as u8
    };
    let mut ok = true;
    for round in 0..64 {
        let key: Vec<u8> = (0..if round % 2 == 0 { 16 } else { 32 }).map(|_| next()).collect();
        let aes = crypto::aes::Aes::new(&key).expect("key");
        let mut blocks = [[0u8; 16]; 4];
        for blk in blocks.iter_mut() {
            blk.iter_mut().for_each(|x| *x = next());
        }
        let plain = blocks;
        aes.encrypt4(&mut blocks);
        let mut one = blocks[1];
        aes.decrypt(&mut one);
        aes.decrypt4(&mut blocks);
        ok &= blocks == plain && one == plain[1];
    }
    t.check("aes decrypt(encrypt(x)) == x (64 random keys, AES-128 and AES-256)", ok);
}

/// pow_ct against pow and the Diffie-Hellman of the Secret Service's 1024-bit MODP group
/// (RFC 2409 group 2; the answers from Python's pow)
fn dh_pow(t: &mut T) {
    use crypto::bn::{Mont, from_be, to_be};
    let p = h("ffffffffffffffffc90fdaa22168c234c4c6628b80dc1cd129024e088a67cc74020bbea63b139b22514a08798e3404ddef9519b3cd3a431b302b0a6df25f14374fe1356d6d51c245e485b576625e7ec6f44c42e9a637ed6b0bff5cb6f406b7edee386bfb5a899fa5ae9f24117c4b1fe649286651ece65381ffffffffffffffff");
    let m = Mont::new(&from_be(&p, 16)).expect("odd prime");
    let x: Vec<u8> = (1..=128u8).collect();
    let y: Vec<u8> = (0..128u32).map(|i| (i * 7 + 3) as u8).collect();
    let px = to_be(&m.pow_ct(&[2], &x), 128);
    t.eq("dh-1024 public key (pow_ct)", hex(&px), String::from("52893c96a07492593d36d00328ba21101c0ab104717cf1928b3890e1ecbd76e827e7ba7d589a03ef64f8f5648cea2b077409319013d832c41a04a3793eb4b73ca2a36306241de180ec36f5cfbef207ba2e04a39f1384dce12b8ebe27b54dbf049be0abc80ebd4849976974b2be955c2c87ba11712576887f5b96427a1d8fb2fd"));
    let py = m.pow_ct(&[2], &y);
    let shared = to_be(&m.pow_ct(&py, &x), 128);
    t.eq("dh-1024 shared secret (pow_ct)", hex(&shared), String::from("814cadb994e653363e8ce3837052aa7052cf046dd4f5bab2d561518a660af699fd0858e7cb4b3c1578aa2c0ddcb06553e85115a53e2c2befd6c9f8eeb816e4fd1e547374bc36a934e2e28678d60559984b2c5a4754d60bd105f0459c694131b01a35015470547a2ca6ac16fd5e40a5dd91613391455bc84abb37cbe58a7dea13"));
    t.eq("dh-1024 both sides agree", hex(&to_be(&m.pow_ct(&from_be(&px, 16), &y), 128)), hex(&shared));
    let mut key = [0u8; 16];
    crypto::hkdf_sha256(&[], &shared, &[], &mut key);
    t.eq("dh-ietf1024-sha256 aes key (hkdf, no salt, no info)", hex(&key), String::from("d3f57bcf025af223fe1dcb0b005ab0b1"));
    // random bases and exponents, also short and zero ones
    let mut g = 0xD1F7u64;
    let mut next = || {
        g = g.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (g >> 33) as u8
    };
    let mut ok = true;
    for round in 0..24usize {
        let base: Vec<u8> = (0..127).map(|_| next()).collect();
        let exp: Vec<u8> = (0..[0, 1, 3, 32, 128][round % 5]).map(|_| next()).collect();
        let b = from_be(&base, 16);
        ok &= m.pow_ct(&b, &exp) == m.pow(&b, &exp);
    }
    t.check("pow_ct == pow (random bases, exponents of 0 to 128 bytes)", ok);
}

/// SHA-384, AES, AES-GCM, RSA and ECDSA (the primitives of the general TLS client)
fn web_pki(t: &mut T) {
    t.eq("sha384 abc", hex(&crypto::sha384(b"abc")), String::from("cb00753f45a35e8bb5a03d699ac65007272c32ab0eded1631a8b605a43ff5bed8086072ba1e7cc2358baeca134c825a7"));
    t.eq("sha384 empty", hex(&crypto::sha384(b"")), String::from("38b060a751ac96384cd9327eb1b1e36a21fdb71114be07434c0cc7bf63f6e1da274edebfe76f65fbd51ad2f14898b95b"));
    let mut b = [0u8; 16];
    b.copy_from_slice(&h("00112233445566778899aabbccddeeff"));
    crypto::aes::Aes::new(&h("000102030405060708090a0b0c0d0e0f")).expect("key").encrypt(&mut b);
    t.eq("aes-128 fips197", hex(&b), String::from("69c4e0d86a7b0430d8cdb78070b4c55a"));
    b.copy_from_slice(&h("00112233445566778899aabbccddeeff"));
    crypto::aes::Aes::new(&(0..32u8).collect::<Vec<u8>>()).expect("key").encrypt(&mut b);
    t.eq("aes-256 fips197", hex(&b), String::from("8ea2b7ca516745bfeafc49904b496089"));
    aes_decrypt(t);
    dh_pow(t);
    let seal =|key: &[u8], iv: &[u8], p: &[u8], a: &[u8]| {
        let g = crypto::gcm::AesGcm::new(key).expect("key");
        let mut n = [0u8; 12];
        n.copy_from_slice(iv);
        let mut d = p.to_vec();
        let tag = g.seal_in_place(&n, a, &mut d);
        d.extend_from_slice(&tag);
        hex(&d)
    };
    t.eq("gcm case 1", seal(&[0; 16], &[0; 12], &[], &[]), String::from("58e2fccefa7e3061367f1d57a4e7455a"));
    t.eq("gcm case 2", seal(&[0; 16], &[0; 12], &[0; 16], &[]), String::from("0388dace60b6a392f328c2b971b2fe78ab6e47d42cec13bdf53a67b21257bddf"));
    let (k, iv) = (h("feffe9928665731c6d6a8f9467308308"), h("cafebabefacedbaddecaf888"));
    let p = h("d9313225f88406e5a55909c5aff5269a86a7a9531534f7da2e4c303d8a318a721c3c0c95956809532fcf0e2449a6b525b16aedf5aa0de657ba637b39");
    let a = h("feedfacedeadbeeffeedfacedeadbeefabaddad2");
    t.eq("gcm case 4", seal(&k, &iv, &p, &a), String::from("42831ec2217774244b7221b784d0d49ce3aa212f2c02a4e035c17e2329aca12e21d514b25466931c7d8f6a5aac84aa051ba30b396a0aac973d58e0915bc94fbc3221a5db94fae95ae7121a47"));
    t.eq("gcm aes-256", seal(&[k.clone(), k.clone()].concat(), &iv, &p, &a), String::from("522dc1f099567d07f47f37a32a84427d643a8cdcbfe5c0c97598a2bd2555d1aa8cb08e48590dbb3da7b08b1056828838c5f61e6393ba7a0abcc9f66276fc6ece0f4e1768cddf8853bb2d551b"));
    // open: authentic, then a flipped bit anywhere
    let g = crypto::gcm::AesGcm::new(&k).expect("key");
    let mut n = [0u8; 12];
    n.copy_from_slice(&iv);
    let ct = h("42831ec2217774244b7221b784d0d49ce3aa212f2c02a4e035c17e2329aca12e21d514b25466931c7d8f6a5aac84aa051ba30b396a0aac973d58e0915bc94fbc3221a5db94fae95ae7121a47");
    let (body, tag) = ct.split_at(ct.len() - 16);
    let mut tg = [0u8; 16];
    tg.copy_from_slice(tag);
    let mut d = body.to_vec();
    t.check("gcm open", g.open_in_place(&n, &a, &mut d, &tg) && d == p);
    let mut d = body.to_vec();
    d[7] ^= 1;
    t.check("gcm forged", !g.open_in_place(&n, &a, &mut d, &tg));
    // RSA-2048 PKCS #1 v1.5 and PSS (SHA-256)
    let key = crypto::rsa::PublicKey::new(&h("e7cf1231eb2b10bde2ee44dec72d68c3831d0bfbac26466c148e3aa15c2641a18c9eea3340bbfde5fd879c83f438c7a622fa06be2ab494879173ee8fc50d63aed04d50ae6f332da0c8d370ff00bcf1ecf48b389ecb0f1be966309f743d2522e4cb85a48df4e5fcc3bb3fff01d5cdb6c61cb863902f079aa467d8f86eeabd105a49664731554098db3b78b568f8ee3786092e197b0b53cc8eeaa20305dcb3cd22a94b001f2c727d3cff4ef6e07af9ab0499e8eb0c164f59868b48eebb645610626f2b7df799a667499702fedf86120ad2cd7b636f0407242fb1e66ee5434e7b7b169d97b9fdc88ef2bd078f2e524fd666e20ae2a6c0d09014239ffe950a1fc8e1"), &[1, 0, 1]).expect("rsa key");
    let digest = h("f962d3b9f84971b47b246530ed8325e89f444ba0a37d6ae088fab0253bb3b82f");
    let (s1, s2) = (h("1493a51afba1a9aceaa1bd8390ba518e51aa16442b63f025d5bbfeec28c83d39805c1037699939b6bb26e1108b050fdad5d95389bcad22242c3b69c95b89722d420cc606749256b17503411bcf419236d638021ead20ae411323d958d6d343cf19331a12c8dee4aa87937587c3ba1ecfe725bcb7e99086c0960bc14eaf82b6a5304a1065a86cfd9d38eecf2c867f798421c45700679051523351e2d2d4ff388f6c48c5933f0d74399a92d093e2e9a518f21bc030dd35b5906e84e80f1861bbdb0c0add8a5f157f7f71f64e722943352cbf58aced19bbe3abbb9c1a9e8f955493c82f2224ea98e1ef4ac09e8debe779c1df1e343f7256e9cda884ccaece5e04b2"), h("1aa2380a1df51dfde54e48ed4fe4770bdc1445a58897efe95ce4eaf748851dbfaf7061420f2bf528da1c99fad1e9a606eafa0846c18d29155a9140a5489fddf568850279fd4730e57375cbd8717fa43d9f9001ff58ad3403c5ef5ce032b8eca7c14ec2da0505ff777dd1873a1f53e2f706225eba392a1109d5d408c378349994ae88a8802ea62389e591ae1eaeb9395ece2c05691d2b16e81012923b846d8448c616f0fafea6a81982194c27a7c75cfa5e8535c99f02111dc1bb155354eff125a3783933b7e7acd4336817071c4b1bf1f8d4205caebe8dddd0010ff98398ae77d52c5b77dc6dd82cff8b13b92b2e7f067b3e1684f2e7d6f8f4fa8799c56c85c6"));
    t.check("rsa pkcs1", key.verify_pkcs1(crypto::HashAlg::Sha256, &digest, &s1));
    t.check("rsa pss", key.verify_pss(crypto::HashAlg::Sha256, &digest, &s2, Some(32)));
    t.check("rsa pss any salt", key.verify_pss(crypto::HashAlg::Sha256, &digest, &s2, None));
    t.check("rsa pss is not pkcs1", !key.verify_pkcs1(crypto::HashAlg::Sha256, &digest, &s2));
    let mut bad = s1.clone();
    bad[100] ^= 4;
    t.check("rsa forged", !key.verify_pkcs1(crypto::HashAlg::Sha256, &digest, &bad));
    let mut other = digest.clone();
    other[0] ^= 1;
    t.check("rsa other digest", !key.verify_pss(crypto::HashAlg::Sha256, &other, &s2, None));
    // ECDSA P-256 / SHA-256 and P-384 / SHA-384
    use crypto::ecdsa::{verify, Curve};
    let (pt, d, r, s) = (h("04699c816344c676847bdfd883e7116078133ba64e5bc59cad4555cc0ef3668cb96be832e8052a2f561c370ff147c17a22607f6d12b42473edf64627624714fbc9"), h("f962d3b9f84971b47b246530ed8325e89f444ba0a37d6ae088fab0253bb3b82f"), h("012d4fd78affc8adbc6260020051f452a40734064fb63b51171e2dcf5afa4caf"), h("4b7cbb8a86b85ca88927cb2739ac2f3c1710c8e0019fbbf6c4327f7c5e527cc4"));
    t.check("ecdsa p-256", verify(Curve::P256, &pt, &d, &r, &s));
    t.check("ecdsa p-256 swapped", !verify(Curve::P256, &pt, &d, &s, &r));
    let mut q = pt.clone();
    q[40] ^= 1;
    t.check("ecdsa p-256 point off the curve", !verify(Curve::P256, &q, &d, &r, &s));
    let (pt, d, r, s) = (h("0478c4a1e180337341664642cbcdb5231281f7dccdfc0b3629fdfa9964e3172aa80b4d7fb3e43c1a2192f4ddecd7163726ba0a521f37144c7c2571e0431a2120a850bdb6639617c9945e184afe4bcd539be9ff3f263e75302fbf34e669a46d705e"), h("3efe56086b5c10a1b314928c3a729175d0380ec7c05f4b7dfbd4a75384f44fc909e1e00e1c513c624c0f72d72c99b0be"), h("ae54f0ba7ea7e93145996f03ab2489876f02e14da8db5975435f88c3a78bde97130bfe66d527ed546ee7ce58365cb63b"), h("701a9080f0214809134d8c673803aec4da3b36a2433ca0600b55150c18c57cf5d69785986685f418b1de138e4b141100"));
    t.check("ecdsa p-384", verify(Curve::P384, &pt, &d, &r, &s));
    let mut d2 = d.clone();
    d2[47] ^= 0x80;
    t.check("ecdsa p-384 other digest", !verify(Curve::P384, &pt, &d2, &r, &s));
}

fn sha2(t: &mut T) {
    t.eq("sha256 empty", hex(&crypto::sha256(b"")), String::from("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"));
    t.eq("sha256 abc", hex(&crypto::sha256(b"abc")), String::from("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"));
    t.eq("sha256 448 bits", hex(&crypto::sha256(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq")),
         String::from("248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"));
    let mut s = crypto::Sha256::new();
    let chunk = [b'a'; 1000];
    for _ in 0..1000 {
        s.update(&chunk);
    }
    t.eq("sha256 million a", hex(&s.finalize()), String::from("cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"));
    t.eq("sha512 empty", hex(&crypto::sha512(b"")), String::from("cf83e1357eefb8bdf1542850d66d8007d620e4050b5715dc83f4a921d36ce9ce47d0d13c5d85f2b0ff8318d2877eec2f63b931bd47417a81a538327af927da3e"));
    t.eq("sha512 abc", hex(&crypto::sha512(b"abc")), String::from("ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f"));
    t.eq("sha512 896 bits", hex(&crypto::sha512(b"abcdefghbcdefghicdefghijdefghijkefghijklfghijklmghijklmnhijklmnoijklmnopjklmnopqklmnopqrlmnopqrsmnopqrstnopqrstu")),
         String::from("8e959b75dae313da8cf4f72814fc143f8f7779c6eb9f7fa17299aeadb6889018501d289e4900f7e4331b99dec4b5433ac7d329eeb6dd26545e96e55b874be909"));
    // streaming in odd pieces equals one shot
    let data: Vec<u8> = (0..1000u32).map(|i| (i * 7 + 3) as u8).collect();
    let mut s = crypto::Sha512::new();
    for c in data.chunks(37) {
        s.update(c);
    }
    t.eq("sha512 pieces", s.finalize(), crypto::sha512(&data));
}

fn hmac(t: &mut T) {
    let k = [0x0bu8; 20];
    t.eq("hmac-sha256 rfc4231 1", hex(&crypto::hmac_sha256(&k, b"Hi There")), String::from("b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"));
    t.eq("hmac-sha512 rfc4231 1", hex(&crypto::hmac_sha512(&k, b"Hi There")),
         String::from("87aa7cdea5ef619d4ff0b4241a1d6cb02379f4e2ce4ec2787ad0b30545e17cdedaa833b7d6b8a702038b274eaea3f4e4be9d914eeb61f1702e696c203a126854"));
    t.eq("hmac-sha256 rfc4231 2", hex(&crypto::hmac_sha256(b"Jefe", b"what do ya want for nothing?")),
         String::from("5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"));
    t.eq("hmac-sha512 rfc4231 2", hex(&crypto::hmac_sha512(b"Jefe", b"what do ya want for nothing?")),
         String::from("164b7a7bfcf819e2e395fbe73b56e0a387bd64222e831fd610270cd7ea2505549758bf75c05a994a6d034f65f8f0e6fdcaeab1a34d4a6b4b636e070a38bce737"));
    // RFC 4231 case 6: a key longer than the block
    let long = [0xaau8; 131];
    t.eq("hmac-sha256 long key", hex(&crypto::hmac_sha256(&long, b"Test Using Larger Than Block-Size Key - Hash Key First")),
         String::from("60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54"));
    let mut okm = [0u8; 42];
    crypto::hkdf_sha256(&h("000102030405060708090a0b0c"), &[0x0b; 22], &h("f0f1f2f3f4f5f6f7f8f9"), &mut okm);
    t.eq("hkdf-sha256 rfc5869 1", hex(&okm), String::from("3cb25f25faacd57a90434f64d0362f2a2d2d0a90cf1a5a4c5db02d56ecc4c5bf34007208d5b887185865"));
    let mut dk = [0u8; 64];
    crypto::pbkdf2_sha256(b"passwd", b"salt", 1, &mut dk);
    t.eq("pbkdf2-sha256 rfc7914 1", hex(&dk),
         String::from("55ac046e56e3089fec1691c22544b605f94185216dde0465e68b9d57c20dacbc49ca9cccf179b645991664b39d77ef317c71b845b1e30bd509112041d3a19783"));
    crypto::pbkdf2_sha256(b"Password", b"NaCl", 80000, &mut dk);
    t.eq("pbkdf2-sha256 rfc7914 2", hex(&dk),
         String::from("4ddcd8f60b98be21830cee5ef22701f9641a4418d04c0414aeff08876b34ab56a1d425a1225833549adb841b51c9b3176a272bdebba1d078478f62b397f33c8d"));
}

fn blake2(t: &mut T) {
    let mut out = [0u8; 64];
    blake2b::blake2b(&mut out, &[], &[b"abc"]);
    t.eq("blake2b-512 abc", hex(&out),
         String::from("ba80a53f981c4d0d6a2797b69f12f6e94c212f14685ac4b74b12bb6fdbffa2d17d87c5392aab792dc252d5de4533cc9518d38aa8dbf1925ab92386edd4009923"));
    blake2b::blake2b(&mut out, &[], &[b""]);
    t.eq("blake2b-512 empty", hex(&out),
         String::from("786a02f742015903c6c6fd852552d272912f4740e15847618a86e217f71f5419d25e1031afee585313896444934eb04b903a685b1448b755d56f701afe9be2ce"));
    // a full last block stays for finalize (exactly 128 and 256 bytes) == split feeding
    let data = [0x5au8; 256];
    let mut a = [0u8; 32];
    let mut b = [0u8; 32];
    blake2b::blake2b(&mut a, b"key", &[&data]);
    blake2b::blake2b(&mut b, b"key", &[&data[..128], &data[128..]]);
    t.eq("blake2b block boundary", a, b);
}

/// the BLAKE3 test-vector input: bytes 0, 1, .. 250, 0, 1, ..
fn b3_input(n: usize) -> Vec<u8> {
    (0..n).map(|i| (i % 251) as u8).collect()
}

fn blake3_vectors(t: &mut T) {
    t.eq("blake3 empty", hex(&blake3::hash(b"")), String::from("af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262"));
    t.eq("blake3 abc", hex(&blake3::hash(b"abc")), String::from("6437b3ac38465133ffb63b75273a8db548c558465d79db03fd359c6cd5bd9d85"));
    // official vectors: the first 32 output bytes for chunk and tree boundaries
    for (n, want) in [
        (1024usize, "42214739f095a406f3fc83deb889744ac00df831c10daa55189b5d121c855af7"),
        (1025, "d00278ae47eb27b34faecf67b4fe263f82d5412916c1ffd97c8cb7fb814b8444"),
        (2048, "e776b6028c7cd22a4d0ba182a8bf62205d2ef576467e838ed6f2529b85fba24a"),
        (2049, "5f4d72f40d7a5f82b15ca2b2e44b1de3c2ef86c426c95c1af0b6879522563030"),
        (3072, "b98cb0ff3623be03326b373de6b9095218513e64f1ee2edd2525c7ad1e5cffd2"),
        (3073, "7124b49501012f81cc7f11ca069ec9226cecb8a2c850cfe644e327d22d3e1cd3"),
        (4096, "015094013f57a5277b59d8475c0501042c0b642e531b0a1c8f58d2163229e969"),
        (4097, "9b4052b38f1c5fc8b1f9ff7ac7b27cd242487b3d890d15c96a1c25b8aa0fb995"),
        (5120, "9cadc15fed8b5d854562b26a9536d9707cadeda9b143978f319ab34230535833"),
        (31744, "62b6960e1a44bcc1eb1a611a8d6235b6b4b78f32e7abc4fb4c6cdcce94895c47"),
        (102400, "bc3e3d41a1146b069abffad3c0d44860cf664390afce4d9661f7902e7943e085"),
    ] {
        t.eq(&alloc::format!("blake3 len {}", n), hex(&blake3::hash(&b3_input(n))), String::from(want));
    }
    // the 131-byte extended output of the empty input
    let mut x = [0u8; 131];
    blake3::Blake3::new().finalize_xof(&mut x);
    t.eq("blake3 xof empty", hex(&x), String::from(
        "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262e00f03e7b69af26b7faaf09fcd333050338ddfe085b8cc869ca98b206c08243a26f5487789e8f660afe6c99ef9e0c52b92e7393024a80459cf91f476f9ffdbda7001c22e159b402631f277ca96f2defdf1078282314e763699a31c5363165421cce14d"));
    let mut tail = [0u8; 40];
    blake3::Blake3::new().finalize_xof_at(91, &mut tail);
    t.eq("blake3 xof seek", &tail[..], &x[91..]);
    let key = *b"whats the Elvish word for friend";
    t.eq("blake3 keyed empty", hex(&blake3::keyed_hash(&key, b"")), String::from("92b2b75604ed3c761f9d6f62392c8a9227ad0ea3f09573e783f1498a4ed60d26"));
    t.eq("blake3 derive empty", hex(&blake3::derive_key("BLAKE3 2019-12-27 16:29:52 test vectors context", b"")),
         String::from("2cc39783c223154fea8dfb7c1b1660f2ac2dcbd1c1de8277b0b0dd39b7e50d7d"));
    // feeding in odd pieces across chunk boundaries
    let data = b3_input(9000);
    let mut s = blake3::Blake3::new();
    for c in data.chunks(333) {
        s.update(c);
    }
    t.eq("blake3 pieces", s.finalize(), blake3::hash(&data));
}

fn chacha_poly(t: &mut T) {
    let key: [u8; 32] = core::array::from_fn(|i| i as u8);
    let mut n12 = [0u8; 12];
    n12[7] = 0x4a;
    let mut buf = SUNSCREEN.to_vec();
    chacha::xor(&key, 1, &n12, &mut buf);
    t.eq("chacha20 rfc8439 2.4.2", hex(&buf), String::from(
        "6e2e359a2568f98041ba0728dd0d6981e97e7aec1d4360c20a27afccfd9fae0bf91b65c5524733ab8f593dabcd62b3571639d624e65152ab8f530c359f0861d807ca0dbf500d6a6156a38e088a22b65e52bc514d16ccf806818ce91ab77937365af90bbf74a35be6b40b8eedf2785e42874d"));
    let tag = poly1305::poly1305(&a32("85d6be7857556d337f4452fe42d506a80103808afb0db2fd4abff6af4149f51b"), b"Cryptographic Forum Research Group");
    t.eq("poly1305 rfc8439 2.5.2", hex(&tag), String::from("a8061dc1305136c6c22b8baf0c0127a9"));
    let akey: [u8; 32] = core::array::from_fn(|i| 0x80 + i as u8);
    let mut anonce = [0u8; 12];
    anonce.copy_from_slice(&h("070000004041424344454647"));
    let aad = h("50515253c0c1c2c3c4c5c6c7");
    let sealed = aead::seal(&akey, &anonce, &aad, SUNSCREEN);
    t.eq("chacha20-poly1305 rfc8439 2.8.2", hex(&sealed), String::from(
        "d31a8d34648e60db7b86afbc53ef7ec2a4aded51296e08fea9e2b5a736ee62d63dbea45e8ca9671282fafb69da92728b1a71de0a9e060b2905d6a5b67ecd3b3692ddbd7f2d778b8c9803aee328091b58fab324e4fad675945585808b4831d7bc3ff4def08e4b7a9de576d26586cec64b61161ae10b594f09e26a7e902ecbd0600691"));
    t.eq("chacha20-poly1305 open", aead::open(&akey, &anonce, &aad, &sealed).as_deref(), Some(SUNSCREEN));
    let mut forged = sealed.clone();
    forged[5] ^= 1;
    t.check("chacha20-poly1305 forged", aead::open(&akey, &anonce, &aad, &forged).is_none());
    t.check("chacha20-poly1305 wrong aad", aead::open(&akey, &anonce, b"x", &sealed).is_none());
    let mut hn = [0u8; 16];
    hn.copy_from_slice(&h("000000090000004a0000000031415927"));
    t.eq("hchacha20", hex(&chacha::hchacha20(&key, &hn)), String::from("82413b4227b27bfed30e42508a877d73a0f9e4d58a74a853c12ec41326d3ecdc"));
    let mut xn = [0u8; 24];
    xn.copy_from_slice(&h("404142434445464748494a4b4c4d4e4f5051525354555657"));
    let xs = aead::x_seal(&akey, &xn, &aad, SUNSCREEN);
    t.eq("xchacha20-poly1305 draft", hex(&xs), String::from(
        "bd6d179d3e83d43b9576579493c0e939572a1700252bfaccbed2902c21396cbb731c7f1b0b4aa6440bf3a82f4eda7e39ae64c6708c54c216cb96b72e1213b4522f8c9ba40db5d945b11b69b982c1bb9e3f3fac2bc369488f76b2383565d3fff921f9664c97637da9768812f615c68b13b52ec0875924c1c7987947deafd8780acf49"));
    t.eq("xchacha20-poly1305 open", aead::x_open(&akey, &xn, &aad, &xs).as_deref(), Some(SUNSCREEN));
}

fn curve(t: &mut T) {
    let a = a32("77076d0a7318a57d3c16c17251b26645df4c2f87ebc0992ab177fba51db92c2a");
    let b = a32("5dab087e624a8a4b79e17f8b83800ee66f3bb1292618b6fd1c2f8b27ff88e0eb");
    let ap = x25519::public_key(&a);
    let bp = x25519::public_key(&b);
    t.eq("x25519 alice public", hex(&ap), String::from("8520f0098930a754748b7ddcb43ef75a0dbf3a0d26381af4eba4a98eaa9b4e6a"));
    t.eq("x25519 bob public", hex(&bp), String::from("de9edb7d7b7dc1b4d35b61c2ece435373f8343c85b78674dadfc7e146f882b4f"));
    let s1 = x25519::shared(&a, &bp).unwrap_or_default();
    let s2 = x25519::shared(&b, &ap).unwrap_or_default();
    t.eq("x25519 shared", hex(&s1), String::from("4a5d9d5ba4ce2de1728e3bf480350f25e07e21c947d19e3376f09b3c1e161742"));
    t.eq("x25519 shared both ways", s1, s2);
    t.eq("x25519 rfc7748 5.2", hex(&x25519::x25519(&a32("a546e36bf0527c9d3b16154b82465edd62144c0ac1fc5a18506a2244ba449ac4"),
                                                      &a32("e6db6867583030db3594c1a424b15f7c726624ec26b3353b10a903a6d0ab1c4c"))),
         String::from("c3da55379de9c6908e94ea4df28d084f32eccf03491c71f754b4075577a28552"));
    t.check("x25519 low order rejected", x25519::shared(&a, &[0u8; 32]).is_none());
    for (seed, public, msg, sig) in [
        ("9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60", "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a", "",
         "e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e065224901555fb8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b"),
        ("4ccd089b28ff96da9db6c346ec114e0f5b8a319f35aba624da8cf6ed4fb8a6fb", "3d4017c3e843895a92b70aa74d1b7ebc9c982ccf2ec4968cc0cd55f12af4660c", "72",
         "92a009a9f0d4cab8720e820b5f642540a2b27b5416503f8fb3762223ebdb69da085ac1e43e15996e458f3613d0f11d8c387b2eaeb4302aeeb00d291612bb0c00"),
        ("c5aa8df43f9f837bedb7442f31dcb7b166d38535076f094b85ce3a2e0b4458f7", "fc51cd8e6218a1a38da47ed00230f0580816ed13ba3303ac5deb911548908025", "af82",
         "6291d657deec24024827e69c3abe01a30ce548a284743a445e3680d7db5ac3ac18ff9b538d16f290ae67f760984dc6594a7c15e9716ed28dc027beceea1ec40a"),
    ] {
        let kp = ed25519::Keypair::from_seed(&a32(seed));
        t.eq("ed25519 public", hex(&kp.public), String::from(public));
        let m = h(msg);
        let s = kp.sign(&m);
        t.eq("ed25519 signature", hex(&s), String::from(sig));
        t.check("ed25519 verify", ed25519::verify(&kp.public, &m, &s));
        let mut bad = s;
        bad[10] ^= 4;
        t.check("ed25519 forged R", !ed25519::verify(&kp.public, &m, &bad));
        let mut bad = s;
        bad[40] ^= 1;
        t.check("ed25519 forged S", !ed25519::verify(&kp.public, &m, &bad));
        t.check("ed25519 other message", !ed25519::verify(&kp.public, b"x", &s));
    }
    // S + L (non-canonical) must be rejected
    let kp = ed25519::Keypair::from_seed(&[7u8; 32]);
    let s = kp.sign(b"m");
    let l = h("edd3f55c1a631258d69cf7a2def9de1400000000000000000000000000000010");
    let mut big = s;
    let mut carry = 0u16;
    for i in 0..32 {
        let v = big[32 + i] as u16 + l[i] as u16 + carry;
        big[32 + i] = v as u8;
        carry = v >> 8;
    }
    t.check("ed25519 S + L rejected", carry == 0 && !ed25519::verify(&kp.public, b"m", &big));
    // a non-canonical public key (y = p) is rejected
    let p = h("edffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff7f");
    let mut pk = [0u8; 32];
    pk.copy_from_slice(&p);
    t.check("ed25519 y = p rejected", !ed25519::valid_public_key(&pk));
    // sealbox
    let rnd: [u8; 56] = core::array::from_fn(|i| (i * 13 + 1) as u8);
    let sealed = sealbox::seal(&bp, &rnd, b"kit", b"repository key").unwrap_or_default();
    t.eq("sealbox open", sealbox::open(&b, b"kit", &sealed).as_deref(), Some(&b"repository key"[..]));
    t.check("sealbox other key", sealbox::open(&a, b"kit", &sealed).is_none());
    t.check("sealbox other aad", sealbox::open(&b, b"x", &sealed).is_none());
}

fn argon(t: &mut T) {
    // RFC 9106 5.3 (4 lanes, 3 passes, secret and associated data)
    let p = argon2::Params { m_kib: 32, t: 3, p: 4, out_len: 32 };
    let mut out = [0u8; 32];
    let ok = argon2::argon2id_ex(&[1u8; 32], &[2u8; 16], &[3u8; 8], &[4u8; 12], &p, &mut out);
    t.check("argon2id parameters", ok);
    t.eq("argon2id rfc9106", hex(&out), String::from("0d640df58d78766c08c037a34a8b53c9d01ef0452d75b65eb52520e96b01e659"));
    let small = argon2::Params { m_kib: 64, t: 2, p: 1, out_len: 32 };
    let phc = argon2::hash(b"correct horse battery", b"0123456789abcdef", &small).unwrap_or_default();
    t.check("argon2id phc prefix", phc.starts_with("$argon2id$v=19$m=64,t=2,p=1$"));
    t.check("argon2id verify", argon2::verify(&phc, b"correct horse battery"));
    t.check("argon2id wrong password", !argon2::verify(&phc, b"correct horse batterz"));
    t.check("argon2id bad string", !argon2::verify("$argon2i$v=19$m=64,t=2,p=1$AAAA$AAAA", b"x"));
}

fn misc(t: &mut T) {
    t.eq("b64", b64::encode(b"foobar"), String::from("Zm9vYmFy"));
    t.eq("b64 pad", b64::encode(b"fo"), String::from("Zm8="));
    t.eq("b64 nopad", b64::encode_nopad(b"f"), String::from("Zg"));
    t.eq("b64 url", b64::encode_url(&[0xfb, 0xff]), String::from("-_8"));
    t.eq("b64 decode", b64::decode("Zm9vYmE=").as_deref(), Some(&b"fooba"[..]));
    t.eq("b64 decode nopad", b64::decode("Zm9vYg").as_deref(), Some(&b"foob"[..]));
    t.check("b64 non-canonical", b64::decode("Zh").is_none());
    t.check("ct_eq", crypto::ct_eq(b"abc", b"abc") && !crypto::ct_eq(b"abc", b"abd") && !crypto::ct_eq(b"ab", b"abc"));
}

/// throughput of the bulk primitives (MB/s, the platform clock)
pub fn bench() {
    let data = alloc::vec![0x42u8; 16 << 20];
    let mb = data.len() as f64 / 1e6;
    let time = |name: &str, f: &mut dyn FnMut()| {
        let t0 = plat::time_us();
        f();
        let dt = (plat::time_us() - t0).max(1) as f64 / 1e6;
        plat::write_stdout(alloc::format!("{:<22} {:>8.0} MB/s\n", name, mb / dt).as_bytes());
    };
    time("blake3", &mut || {
        core::hint::black_box(blake3::hash(&data));
    });
    time("sha256", &mut || {
        core::hint::black_box(crypto::sha256(&data));
    });
    time("sha512", &mut || {
        core::hint::black_box(crypto::sha512(&data));
    });
    let key = [7u8; 32];
    let nonce = [9u8; 24];
    let mut buf = data.clone();
    time("xchacha20-poly1305", &mut || {
        core::hint::black_box(aead::x_seal_in_place(&key, &nonce, b"", &mut buf));
    });
    let gcm = crypto::gcm::AesGcm::new(&key[..16]).expect("key");
    time("aes-128-gcm", &mut || {
        core::hint::black_box(gcm.seal_in_place(&[1; 12], b"", &mut buf));
    });
    let t0 = plat::time_us();
    let mut out = [0u8; 32];
    argon2::argon2id(b"password", b"saltsaltsalt", &argon2::Params::INTERACTIVE, &mut out);
    plat::write_stdout(alloc::format!("argon2id 19 MiB t=2     {:>6} ms\n", (plat::time_us() - t0) / 1000).as_bytes());
    let kp = ed25519::Keypair::from_seed(&key);
    let t0 = plat::time_us();
    let s = kp.sign(b"message");
    let t1 = plat::time_us();
    core::hint::black_box(ed25519::verify(&kp.public, b"message", &s));
    let t2 = plat::time_us();
    plat::write_stdout(alloc::format!("ed25519 sign / verify   {:>6} / {} us\n", t1 - t0, t2 - t1).as_bytes());
    // the public-key checks of a TLS handshake with web PKI certificates
    let n: Vec<u8> = (0..256).map(|i| if i == 0 { 0xC5 } else if i == 255 { 0x0B } else { (i * 37) as u8 }).collect();
    let rk = crypto::rsa::PublicKey::new(&n, &[1, 0, 1]).expect("rsa");
    let t0 = plat::time_us();
    core::hint::black_box(rk.verify_pkcs1(crypto::HashAlg::Sha256, &[0; 32], &[0x11; 256]));
    plat::write_stdout(alloc::format!("rsa-2048 verify         {:>6} us\n", plat::time_us() - t0).as_bytes());
    let g = "046b17d1f2e12c4247f8bce6e563a440f277037d812deb33a0f4a13945d898c2964fe342e2fe1a7f9b8ee7eb4a7c0f9e162bce33576b315ececbb6406837bf51f5";
    let t0 = plat::time_us();
    core::hint::black_box(crypto::ecdsa::verify(crypto::ecdsa::Curve::P256, &h(g), &[1; 32], &[2; 32], &[3; 32]));
    plat::write_stdout(alloc::format!("ecdsa p-256 verify      {:>6} us\n", plat::time_us() - t0).as_bytes());
}

/// answer the cases of a vector file (tools/crypto_check.py): one case per line,
/// "alg hex hex ..." -> one line "hex" per case ("-" for None)
pub fn vectors(path: &str) -> i32 {
    let Some(text) = plat::read_file(path) else {
        plat::write_stderr(b"coretest: cannot read the vector file\n");
        return 2;
    };
    let text = String::from_utf8_lossy(&text);
    let mut out = String::new();
    for line in text.lines() {
        let mut f = line.split(' ');
        let alg = f.next().unwrap_or("");
        let args: Vec<Vec<u8>> = f.map(|x| unhex(x).unwrap_or_default()).collect();
        let arg = |i: usize| args.get(i).map(|v| v.as_slice()).unwrap_or(&[]);
        let k32 = |i: usize| -> [u8; 32] {
            let mut k = [0u8; 32];
            let a = arg(i);
            k[..a.len().min(32)].copy_from_slice(&a[..a.len().min(32)]);
            k
        };
        let r: Option<Vec<u8>> = match alg {
            "sha256" => Some(crypto::sha256(arg(0)).to_vec()),
            "sha512" => Some(crypto::sha512(arg(0)).to_vec()),
            "hmac256" => Some(crypto::hmac_sha256(arg(0), arg(1)).to_vec()),
            "hmac512" => Some(crypto::hmac_sha512(arg(0), arg(1)).to_vec()),
            "hkdf256" => {
                let mut o = alloc::vec![0u8; arg(3).first().copied().unwrap_or(32) as usize];
                crypto::hkdf_sha256(arg(0), arg(1), arg(2), &mut o);
                Some(o)
            }
            "blake2b" => {
                let mut o = alloc::vec![0u8; arg(2).first().copied().unwrap_or(64) as usize];
                blake2b::blake2b(&mut o, arg(0), &[arg(1)]);
                Some(o)
            }
            "aead" => {
                let mut n = [0u8; 12];
                n.copy_from_slice(arg(1));
                Some(aead::seal(&k32(0), &n, arg(2), arg(3)))
            }
            "x25519" => Some(x25519::x25519(&k32(0), &k32(1)).to_vec()),
            "ed25519pub" => Some(ed25519::Keypair::from_seed(&k32(0)).public.to_vec()),
            "ed25519sign" => Some(ed25519::Keypair::from_seed(&k32(0)).sign(arg(1)).to_vec()),
            "sha384" => Some(crypto::sha384(arg(0)).to_vec()),
            "aesgcm" => {
                let mut n = [0u8; 12];
                n.copy_from_slice(arg(1));
                crypto::gcm::AesGcm::new(arg(0)).map(|g| {
                    let mut d = arg(3).to_vec();
                    let tag = g.seal_in_place(&n, arg(2), &mut d);
                    d.extend_from_slice(&tag);
                    d
                })
            }
            "aesgcmopen" => {
                let mut n = [0u8; 12];
                n.copy_from_slice(arg(1));
                let ct = arg(3);
                crypto::gcm::AesGcm::new(arg(0)).and_then(|g| {
                    let (body, tag) = ct.split_at(ct.len() - 16);
                    let mut t = [0u8; 16];
                    t.copy_from_slice(tag);
                    let mut d = body.to_vec();
                    g.open_in_place(&n, arg(2), &mut d, &t).then_some(d)
                })
            }
            "rsapkcs1" | "rsapss" => {
                // n e hash(1=256,2=384,3=512) digest sig
                let h = match arg(2).first() {
                    Some(2) => crypto::HashAlg::Sha384,
                    Some(3) => crypto::HashAlg::Sha512,
                    _ => crypto::HashAlg::Sha256,
                };
                let ok = crypto::rsa::PublicKey::new(arg(0), arg(1)).is_some_and(|k| {
                    if alg == "rsapkcs1" { k.verify_pkcs1(h, arg(3), arg(4)) } else { k.verify_pss(h, arg(3), arg(4), Some(h.len())) }
                });
                Some(alloc::vec![ok as u8])
            }
            "p256pub" => Some(crypto::p256::public_key(&k32(0)).to_vec()),
            "p256ecdh" => crypto::p256::shared(&k32(0), arg(1)).map(|v| v.to_vec()),
            "ecdsa" => {
                // curve(1=P-256, 2=P-384) point digest r s
                let c = if arg(0).first() == Some(&2) { crypto::ecdsa::Curve::P384 } else { crypto::ecdsa::Curve::P256 };
                Some(alloc::vec![crypto::ecdsa::verify(c, arg(1), arg(2), arg(3), arg(4)) as u8])
            }
            "ed25519verify" => {
                let mut s = [0u8; 64];
                s.copy_from_slice(arg(2));
                Some(alloc::vec![ed25519::verify(&k32(0), arg(1), &s) as u8])
            }
            _ => None,
        };
        out.push_str(&r.map_or(String::from("-"), |v| hex(&v)));
        out.push('\n');
    }
    plat::write_stdout(out.as_bytes());
    0
}

/// the primitives of WPA2 / WPA3: SHA-1, HMAC-SHA1, PBKDF2-HMAC-SHA1, AES key wrap, AES-CMAC
fn wpa(t: &mut T) {
    t.eq("sha1 empty", hex(&crypto::sha1(b"")), String::from("da39a3ee5e6b4b0d3255bfef95601890afd80709"));
    t.eq("sha1 abc", hex(&crypto::sha1(b"abc")), String::from("a9993e364706816aba3e25717850c26c9cd0d89d"));
    t.eq("sha1 448 bits", hex(&crypto::sha1(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq")), String::from("84983e441c3bd26ebaae4aa1f95129e5e54670f1"));
    let mut s = crypto::Sha1::new();
    let chunk = [b'a'; 1000];
    for _ in 0..1000 {
        s.update(&chunk);
    }
    t.eq("sha1 million a", hex(&s.finalize()), String::from("34aa973cd4c4daa4f61eeb2bdbad27316534016f"));
    // streaming in odd pieces equals one shot (block boundaries, the 56-byte padding edge)
    let data: Vec<u8> = (0..300u32).map(|i| (i * 11 + 5) as u8).collect();
    for n in [55usize, 56, 63, 64, 65, 300] {
        let mut s = crypto::Sha1::new();
        for c in data[..n].chunks(7) {
            s.update(c);
        }
        t.eq(&alloc::format!("sha1 pieces {}", n), s.finalize(), crypto::sha1(&data[..n]));
    }
    t.eq("hmac-sha1 rfc2202 1", hex(&crypto::hmac_sha1(&[0x0b; 20], b"Hi There")), String::from("b617318655057264e28bc0b6fb378c8ef146be00"));
    t.eq("hmac-sha1 rfc2202 2", hex(&crypto::hmac_sha1(b"Jefe", b"what do ya want for nothing?")), String::from("effcdf6ae5eb2fa2d27416d5f184df9c259a7c79"));
    t.eq("hmac-sha1 long key", hex(&crypto::hmac_sha1(&[0xaa; 80], b"Test Using Larger Than Block-Size Key - Hash Key First")), String::from("aa4ae5e15272d00e95705637ce8a3b55ed402112"));
    let mut dk = [0u8; 20];
    for (c, want) in [(1u32, "0c60c80f961f0e71f3a9b524af6012062fe037a6"), (2, "ea6c014dc72d6f8ccd1ed92ace1d41f0d8de8957"), (4096, "4b007901b765489abead49d926f721d065a429c1")] {
        crypto::pbkdf2_sha1(b"password", b"salt", c, &mut dk);
        t.eq(&alloc::format!("pbkdf2-sha1 rfc6070 c={}", c), hex(&dk), String::from(want));
    }
    // the PSK of WPA2-Personal: 4096 rounds, 256 bits (IEEE 802.11-2020 J.4)
    let mut psk = [0u8; 32];
    crypto::pbkdf2_sha1(b"password", b"IEEE", 4096, &mut psk);
    t.eq("psk J.4 1", hex(&psk), String::from("f42c6fc52df0ebef9ebb4b90b38a5f902e83fe1b135a70e23aed762e9710a12e"));
    crypto::pbkdf2_sha1(b"ThisIsAPassword", b"ThisIsASSID", 4096, &mut psk);
    t.eq("psk J.4 2", hex(&psk), String::from("0dc0d6eb90555ed6419756b9a15ec3e3209b63df707dd508d14581f8982721af"));
    // the generic PBKDF2 gives the same as the one of SHA-256 already there
    let (mut a, mut b) = ([0u8; 40], [0u8; 40]);
    crypto::pbkdf2::<crypto::Sha256>(b"pw", b"salty", 3, &mut a);
    crypto::pbkdf2_sha256(b"pw", b"salty", 3, &mut b);
    t.eq("pbkdf2 generic", a, b);
    // RFC 3394 4.1: a 128-bit key under a 128-bit KEK
    let kek = h("000102030405060708090a0b0c0d0e0f");
    let wrapped = crypto::aes_wrap(&kek, &h("00112233445566778899aabbccddeeff"));
    t.eq("aes key wrap rfc3394 4.1", wrapped.as_deref().map(hex), Some(String::from("1fa68b0a8112b447aef34bd8fb5a7b829d3e862371d2cfe5")));
    if let Some(w) = wrapped {
        t.eq("aes key unwrap", crypto::aes_unwrap(&kek, &w).map(|v| hex(&v)), Some(String::from("00112233445566778899aabbccddeeff")));
        let mut bad = w.clone();
        bad[9] ^= 1;
        t.check("aes key unwrap refuses a changed one", crypto::aes_unwrap(&kek, &bad).is_none());
    }
    let long: Vec<u8> = (0..48u8).collect();
    t.eq("aes key wrap round trip", crypto::aes_wrap(&kek, &long).and_then(|w| crypto::aes_unwrap(&kek, &w)), Some(long.clone()));
    t.check("aes key wrap wants whole blocks", crypto::aes_wrap(&kek, &long[..20]).is_none() && crypto::aes_wrap(&kek, &long[..8]).is_none());
    // RFC 4493 4: AES-CMAC with AES-128
    let k = h("2b7e151628aed2a6abf7158809cf4f3c");
    let m = h("6bc1bee22e409f96e93d7e117393172aae2d8a571e03ac9c9eb76fac45af8e5130c81c46a35ce411e5fbc1191a0a52eff69f2445df4f9b17ad2b417be66c3710");
    for (n, want) in [(0usize, "bb1d6929e95937287fa37d129b756746"), (16, "070a16b46b4d4144f79bdd9dd04a287c"), (40, "dfa66747de9ae63030ca32611497c827"), (64, "51f0bebf7e3b9d92fc49741779363cfe")] {
        t.eq(&alloc::format!("aes-cmac rfc4493 {} bytes", n), crypto::cmac_aes(&k, &m[..n]).map(|c| hex(&c)), Some(String::from(want)));
    }
    t.check("aes-cmac wants a 128-bit key", crypto::cmac_aes(&k[..15], b"x").is_none());
}
