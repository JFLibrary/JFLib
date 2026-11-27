// certs -- the operating system's trusted certificate authorities
//
//   verify_tls_server(chain, host, ocsp, extra, time) -> Option<Result<(), Refused>>
//        Windows: CryptoAPI builds and checks the chain (leaf first, DER) for a TLS server of
//        that name at that time (Unix seconds), with the machine's and user's roots, enterprise
//        policy and automatic root updates (like for every program there, Windows may fetch a
//        missing intermediate or root); the leaf's revocation from the stapled OCSP response
//        (ocsp) and what Windows has cached, nothing fetched for it.  Where its roots give no
//        path, once more with the extra CAs (DER: a company CA) as its only roots, nothing
//        fetched.  macOS and iOS: the Security framework (SecTrustEvaluateWithError) with the
//        system's and the user's roots and trust settings, the extra CAs as anchors beside them,
//        and the stapled response, without network access.  Either way the system's policy
//        applies to the extra CAs as well, and its answer is the whole answer.  Some(Err) says
//        why the system refuses.  Linux: None (netio checks against the bundle, then the extra
//        CAs).
//   verify_tls_server_roots(chain, host, roots, time) -> Option<Result<(), Refused>>
//        the same with only these CAs as trust anchors (DER), not the system's: nothing
//        fetched, no revocation; the system's checks against a PKI of one's own
//        (tools/limbo_check.py --system).  An empty host: no name checked.  Linux: None.
//   Refused { reason, text }   Reason: UnknownCa, Expired, Name, Revoked, Usage, Other; text:
//        the system's own words (and its code)
//   root_bundle() -> Option<Vec<u8>>
//        Linux: PEM -- the file SSL_CERT_FILE names, else the distribution's CA bundle (Debian /
//        Ubuntu, Fedora / RHEL, openSUSE, Alpine), and the files in the folders of SSL_CERT_DIR
//        (':' between them), as OpenSSL takes the two; macOS: /etc/ssl/cert.pem.  Windows,
//        iOS: None.

use alloc::string::String;
use alloc::vec::Vec;

/// why the system refused a server's chain
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Reason {
    /// no path to a CA it trusts: a self-signed certificate, a CA of one's own, a missing
    /// intermediate
    UnknownCa,
    /// a certificate of the chain has expired or is not valid yet
    Expired,
    /// the certificate is not for this name
    Name,
    /// a certificate of the chain is revoked
    Revoked,
    /// a certificate is not for a TLS server (its extended key usage)
    Usage,
    /// another reason (the text says which)
    Other,
}

/// the system's refusal: the reason and the system's own words
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refused {
    pub reason: Reason,
    pub text: String,
}

impl Refused {
    pub fn new(reason: Reason, text: impl Into<String>) -> Refused {
        Refused { reason, text: text.into() }
    }
}

impl core::fmt::Display for Refused {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.text)
    }
}

#[cfg(target_os = "linux")]
pub fn root_bundle() -> Option<Vec<u8>> {
    const PATHS: [&str; 5] = [
        "/etc/ssl/certs/ca-certificates.crt",
        "/etc/pki/tls/certs/ca-bundle.crt",
        "/etc/ssl/ca-bundle.pem",
        "/etc/pki/ca-trust/extracted/pem/tls-ca-bundle.pem",
        "/etc/ssl/cert.pem",
    ];
    let mut pem = match crate::env("SSL_CERT_FILE").filter(|f| !f.is_empty()) {
        Some(f) => crate::read_file_raw(&f),
        None => PATHS.iter().find_map(|p| crate::read_file_raw(p)),
    }
    .unwrap_or_default();
    // a hashed folder (c_rehash) holds links to the same files: each certificate counts once
    // anyway (the parsed anchors are only searched)
    for dir in crate::env("SSL_CERT_DIR").unwrap_or_default().split(':').filter(|d| !d.is_empty()) {
        let mut names: Vec<String> = crate::fs::read_dir(dir).unwrap_or_default().into_iter().map(|e| e.name).collect();
        names.sort();
        for n in names {
            if let Some(b) = crate::read_file_raw(&alloc::format!("{}/{}", dir.trim_end_matches('/'), n)) {
                pem.push(b'\n');
                pem.extend_from_slice(&b);
            }
        }
    }
    (!pem.is_empty()).then_some(pem)
}

#[cfg(windows)]
pub fn root_bundle() -> Option<Vec<u8>> {
    None
}

#[cfg(target_os = "linux")]
pub fn verify_tls_server(_chain: &[&[u8]], _host: &str, _ocsp: Option<&[u8]>, _extra: &[&[u8]], _time: i64) -> Option<Result<(), Refused>> {
    None
}

#[cfg(target_os = "linux")]
pub fn verify_tls_server_roots(_chain: &[&[u8]], _host: &str, _roots: &[&[u8]], _time: i64) -> Option<Result<(), Refused>> {
    None
}

/// the name as the systems take it: no trailing dot, an IPv6 address without brackets
#[cfg(any(windows, target_vendor = "apple"))]
fn bare(host: &str) -> &str {
    let h = host.strip_suffix('.').unwrap_or(host);
    h.strip_prefix('[').and_then(|h| h.strip_suffix(']')).unwrap_or(h)
}

#[cfg(windows)]
mod win {
    use core::ffi::c_void;

    pub const ENCODING: u32 = 0x0000_0001 | 0x0001_0000; // X509_ASN_ENCODING | PKCS_7_ASN_ENCODING
    pub const STORE_PROV_MEMORY: usize = 2;
    pub const STORE_ADD_ALWAYS: u32 = 4;
    pub const CHAIN_POLICY_SSL: usize = 4;
    pub const AUTHTYPE_SERVER: u32 = 2;
    // CertGetCertificateChain: the leaf's revocation (CERT_CHAIN_REVOCATION_CHECK_END_CERT),
    // from what Windows has cached and the stapled response only (..._CACHE_ONLY)
    pub const REVOCATION_END_CERT: u32 = 0x1000_0000;
    pub const REVOCATION_CACHE_ONLY: u32 = 0x8000_0000;
    // nothing fetched (CERT_CHAIN_CACHE_ONLY_URL_RETRIEVAL, CERT_CHAIN_DISABLE_AIA)
    pub const CACHE_ONLY_URL_RETRIEVAL: u32 = 0x0000_0004;
    pub const DISABLE_AIA: u32 = 0x0000_2000;
    // the SSL policy: revocation that is not known (nothing cached, nothing stapled) is no
    // refusal (CERT_CHAIN_POLICY_IGNORE_ALL_REV_UNKNOWN_FLAGS)
    pub const IGNORE_ALL_REV_UNKNOWN: u32 = 0x0000_0F00;
    pub const OCSP_RESPONSE_PROP_ID: u32 = 70;
    pub const SET_PROPERTY_IGNORE_PERSIST_ERROR: u32 = 0x8000_0000;
    // CERT_CHAIN_EXCLUSIVE_ENABLE_CA_FLAG: a CA certificate among the exclusive roots is an
    // anchor too, not only a self-signed one
    pub const EXCLUSIVE_ENABLE_CA: u32 = 0x1;

    #[repr(C)]
    pub struct Blob {
        pub len: u32,
        pub data: *const u8,
    }

    #[repr(C)]
    pub struct FileTime {
        pub low: u32,
        pub high: u32,
    }

    #[repr(C)]
    pub struct CtlUsage {
        pub count: u32,
        pub ids: *const *const u8,
    }

    #[repr(C)]
    pub struct UsageMatch {
        pub ty: u32,
        pub usage: CtlUsage,
    }

    #[repr(C)]
    pub struct ChainPara {
        pub size: u32,
        pub usage: UsageMatch,
    }

    // CERT_CHAIN_ENGINE_CONFIG with the exclusive root store (Windows 8 and later)
    #[repr(C)]
    pub struct EngineConfig {
        pub size: u32,
        pub restricted_root: *mut c_void,
        pub restricted_trust: *mut c_void,
        pub restricted_other: *mut c_void,
        pub additional_count: u32,
        pub additional: *mut *mut c_void,
        pub flags: u32,
        pub url_timeout: u32,
        pub max_cached: u32,
        pub cycle_modulus: u32,
        pub exclusive_root: *mut c_void,
        pub exclusive_people: *mut c_void,
        pub exclusive_flags: u32,
    }

    #[repr(C)]
    pub struct SslExtra {
        pub size: u32,
        pub auth_type: u32,
        pub checks: u32,
        pub server_name: *const u16,
    }

    #[repr(C)]
    pub struct PolicyPara {
        pub size: u32,
        pub flags: u32,
        pub extra: *const c_void,
    }

    #[repr(C)]
    pub struct PolicyStatus {
        pub size: u32,
        pub error: u32,
        pub chain_index: i32,
        pub element_index: i32,
        pub extra: *mut c_void,
    }

    crate::win_dll::system_dll! { "crypt32.dll" {
        pub fn CertCreateCertificateContext(enc: u32, der: *const u8, len: u32) -> *const c_void = core::ptr::null();
        pub fn CertFreeCertificateContext(ctx: *const c_void) -> i32 = 0;
        pub fn CertSetCertificateContextProperty(ctx: *const c_void, prop: u32, flags: u32, data: *const c_void) -> i32 = 0;
        pub fn CertOpenStore(provider: usize, enc: u32, prov: usize, flags: u32, para: *const c_void) -> *mut c_void = core::ptr::null_mut();
        pub fn CertCloseStore(store: *mut c_void, flags: u32) -> i32 = 0;
        pub fn CertAddEncodedCertificateToStore(store: *mut c_void, enc: u32, der: *const u8, len: u32, disposition: u32, out: *mut *const c_void) -> i32 = 0;
        pub fn CertCreateCertificateChainEngine(config: *const EngineConfig, engine: *mut *mut c_void) -> i32 = 0;
        pub fn CertFreeCertificateChainEngine(engine: *mut c_void) = ();
        pub fn CertGetCertificateChain(engine: *mut c_void, ctx: *const c_void, time: *const c_void, extra: *mut c_void, para: *const ChainPara, flags: u32, reserved: *mut c_void, chain: *mut *const c_void) -> i32 = 0;
        pub fn CertFreeCertificateChain(chain: *const c_void) = ();
        pub fn CertVerifyCertificateChainPolicy(policy: usize, chain: *const c_void, para: *const PolicyPara, status: *mut PolicyStatus) -> i32 = 0;
    } }

    /// a memory store with these certificates (null when Windows gives none)
    pub unsafe fn store_of(ders: &[&[u8]]) -> *mut c_void {
        // SAFETY: the caller's; each DER is copied into the store during the call.
        unsafe {
            let store = CertOpenStore(STORE_PROV_MEMORY, 0, 0, 0, core::ptr::null());
            if !store.is_null() {
                for c in ders {
                    CertAddEncodedCertificateToStore(store, ENCODING, c.as_ptr(), c.len() as u32, STORE_ADD_ALWAYS, core::ptr::null_mut());
                }
            }
            store
        }
    }
}

/// the refusal of the SSL policy's status code
#[cfg(windows)]
fn refused_win(code: u32) -> Refused {
    let (reason, text) = match code {
        // CERT_E_UNTRUSTEDROOT, CERT_E_CHAINING, CERT_E_UNTRUSTEDCA, CERT_E_UNTRUSTEDTESTROOT
        0x800B_0109 | 0x800B_010A | 0x800B_0112 | 0x800B_010D => (Reason::UnknownCa, "no path to a trusted root"),
        // CERT_E_EXPIRED
        0x800B_0101 => (Reason::Expired, "a certificate has expired or is not valid yet"),
        // CERT_E_CN_NO_MATCH, CERT_E_INVALID_NAME
        0x800B_010F | 0x800B_0114 => (Reason::Name, "the certificate is not for this name"),
        // CRYPT_E_REVOKED, CERT_E_REVOKED
        0x8009_2010 | 0x800B_010C => (Reason::Revoked, "a certificate is revoked"),
        // CERT_E_WRONG_USAGE
        0x800B_0110 => (Reason::Usage, "a certificate is not for a TLS server"),
        _ => (Reason::Other, "the certificate is refused"),
    };
    Refused::new(reason, alloc::format!("{} (CryptoAPI 0x{:08X})", text, code))
}

#[cfg(windows)]
pub fn verify_tls_server(chain: &[&[u8]], host: &str, ocsp: Option<&[u8]>, extra: &[&[u8]], time: i64) -> Option<Result<(), Refused>> {
    // the extra CAs only where the system's roots give no path (CERT_E_UNTRUSTEDROOT and the
    // like), then as CryptoAPI's only roots: its policy holds for them too, and a chain it
    // refused for another reason (revoked, expired, another name) is not tried again
    Some(match verify_win(chain, host, ocsp, None, time) {
        Err(e) if e.reason == Reason::UnknownCa && !extra.is_empty() => verify_win(chain, host, ocsp, Some(extra), time),
        r => r,
    })
}

#[cfg(windows)]
pub fn verify_tls_server_roots(chain: &[&[u8]], host: &str, roots: &[&[u8]], time: i64) -> Option<Result<(), Refused>> {
    Some(verify_win(chain, host, None, Some(roots), time))
}

#[cfg(windows)]
fn verify_win(chain: &[&[u8]], host: &str, ocsp: Option<&[u8]>, roots: Option<&[&[u8]]>, time: i64) -> Result<(), Refused> {
    use core::ffi::c_void;
    use core::ptr::{null, null_mut};
    use win::*;
    let Some(leaf) = chain.first() else { return Err(Refused::new(Reason::Other, "no certificate")) };
    let server_auth = b"1.3.6.1.5.5.7.3.1\0";
    let ids = [server_auth.as_ptr()];
    let host = bare(host);
    let name: Vec<u16> = host.encode_utf16().chain(core::iter::once(0)).collect();
    // FILETIME: 100 ns since 1601; a time before that or past its end is none CryptoAPI takes
    let Some(ft) = u64::try_from((time as i128 + 11_644_473_600) * 10_000_000).ok() else {
        return Err(Refused::new(Reason::Other, "a time CryptoAPI cannot check at (before 1601)"));
    };
    let ft = FileTime { low: ft as u32, high: (ft >> 32) as u32 };
    // SAFETY: every pointer handed to CryptoAPI points into a buffer that lives until the call
    // returns; contexts, stores, the engine and the chain are freed on every path below.
    unsafe {
        let ctx = CertCreateCertificateContext(ENCODING, leaf.as_ptr(), leaf.len() as u32);
        if ctx.is_null() {
            return Err(Refused::new(Reason::Other, "the certificate cannot be read (CryptoAPI)"));
        }
        // the stapled response, where Windows' revocation check looks first
        if let Some(r) = ocsp.filter(|r| !r.is_empty() && r.len() <= u32::MAX as usize) {
            let blob = Blob { len: r.len() as u32, data: r.as_ptr() };
            CertSetCertificateContextProperty(ctx, OCSP_RESPONSE_PROP_ID, SET_PROPERTY_IGNORE_PERSIST_ERROR, &blob as *const Blob as *const c_void);
        }
        let store = store_of(&chain[1..]);
        // only these roots: an engine of its own, nothing fetched, no revocation
        let mut engine: *mut c_void = null_mut();
        let mut root_store: *mut c_void = null_mut();
        let mut flags = REVOCATION_END_CERT | REVOCATION_CACHE_ONLY;
        if let Some(r) = roots {
            root_store = store_of(r);
            let cfg = EngineConfig {
                size: core::mem::size_of::<EngineConfig>() as u32,
                restricted_root: null_mut(),
                restricted_trust: null_mut(),
                restricted_other: null_mut(),
                additional_count: 0,
                additional: null_mut(),
                flags: CACHE_ONLY_URL_RETRIEVAL | DISABLE_AIA,
                url_timeout: 0,
                max_cached: 0,
                cycle_modulus: 0,
                exclusive_root: root_store,
                exclusive_people: null_mut(),
                exclusive_flags: EXCLUSIVE_ENABLE_CA,
            };
            if root_store.is_null() || CertCreateCertificateChainEngine(&cfg, &mut engine) == 0 || engine.is_null() {
                if !root_store.is_null() {
                    CertCloseStore(root_store, 0);
                }
                if !store.is_null() {
                    CertCloseStore(store, 0);
                }
                CertFreeCertificateContext(ctx);
                return Err(Refused::new(Reason::Other, "no chain engine of these roots (CryptoAPI)"));
            }
            flags = CACHE_ONLY_URL_RETRIEVAL | DISABLE_AIA;
        }
        let para = ChainPara { size: core::mem::size_of::<ChainPara>() as u32, usage: UsageMatch { ty: 0, usage: CtlUsage { count: 1, ids: ids.as_ptr() } } };
        let mut ch: *const c_void = null();
        let ok = CertGetCertificateChain(engine, ctx, &ft as *const FileTime as *const c_void, store, &para, flags, null_mut(), &mut ch) != 0;
        let result = if !ok || ch.is_null() {
            Err(Refused::new(Reason::Other, "no certificate chain (CryptoAPI)"))
        } else {
            let extra = SslExtra { size: core::mem::size_of::<SslExtra>() as u32, auth_type: AUTHTYPE_SERVER, checks: 0, server_name: if host.is_empty() { null() } else { name.as_ptr() } };
            let pp = PolicyPara { size: core::mem::size_of::<PolicyPara>() as u32, flags: IGNORE_ALL_REV_UNKNOWN, extra: &extra as *const SslExtra as *const c_void };
            let mut st = PolicyStatus { size: core::mem::size_of::<PolicyStatus>() as u32, error: 0, chain_index: 0, element_index: 0, extra: null_mut() };
            let r = CertVerifyCertificateChainPolicy(CHAIN_POLICY_SSL, ch, &pp, &mut st);
            CertFreeCertificateChain(ch);
            if r == 0 {
                Err(Refused::new(Reason::Other, "the SSL policy could not be checked (CryptoAPI)"))
            } else if st.error == 0 {
                Ok(())
            } else {
                Err(refused_win(st.error))
            }
        };
        if !engine.is_null() {
            CertFreeCertificateChainEngine(engine);
        }
        if !root_store.is_null() {
            CertCloseStore(root_store, 0);
        }
        if !store.is_null() {
            CertCloseStore(store, 0);
        }
        CertFreeCertificateContext(ctx);
        result
    }
}

/// macOS: the system's PEM bundle (/etc/ssl/cert.pem, updated with the system); iOS: None.  Apple's
/// own trust decision is verify_tls_server.
#[cfg(target_vendor = "apple")]
pub fn root_bundle() -> Option<Vec<u8>> {
    if cfg!(target_os = "macos") { crate::read_file_raw("/etc/ssl/cert.pem") } else { None }
}

#[cfg(target_vendor = "apple")]
pub fn verify_tls_server(chain: &[&[u8]], host: &str, ocsp: Option<&[u8]>, extra: &[&[u8]], time: i64) -> Option<Result<(), Refused>> {
    Some(apple::verify(chain, host, ocsp, apple::Anchors::System(extra), time))
}

#[cfg(target_vendor = "apple")]
pub fn verify_tls_server_roots(chain: &[&[u8]], host: &str, roots: &[&[u8]], time: i64) -> Option<Result<(), Refused>> {
    Some(apple::verify(chain, host, None, apple::Anchors::Only(roots), time))
}

/// macOS and iOS: the Security framework builds and checks the chain (leaf first, DER) for a TLS
/// server of that name, with the system's and the user's roots and trust settings (or only the
/// roots given), without network access (SecTrustSetNetworkFetchAllowed false)
#[cfg(target_vendor = "apple")]
mod apple {
    use super::{bare, Reason, Refused};
    use alloc::string::String;
    use alloc::vec::Vec;
    use core::ffi::c_void;

    type CfRef = *const c_void;
    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        static kCFTypeArrayCallBacks: [usize; 5];
        fn CFDataCreate(alloc: CfRef, bytes: *const u8, n: isize) -> CfRef;
        fn CFDateCreate(alloc: CfRef, at: f64) -> CfRef;
        fn CFStringCreateWithBytes(alloc: CfRef, bytes: *const u8, n: isize, encoding: u32, external: u8) -> CfRef;
        fn CFArrayCreate(alloc: CfRef, values: *const CfRef, n: isize, callbacks: *const [usize; 5]) -> CfRef;
        fn CFErrorCopyDescription(e: CfRef) -> CfRef;
        fn CFErrorGetCode(e: CfRef) -> isize;
        fn CFStringGetCString(s: CfRef, buf: *mut u8, n: isize, encoding: u32) -> u8;
        fn CFRelease(r: CfRef);
    }
    #[link(name = "Security", kind = "framework")]
    unsafe extern "C" {
        fn SecCertificateCreateWithData(alloc: CfRef, data: CfRef) -> CfRef;
        fn SecPolicyCreateSSL(server: u8, host: CfRef) -> CfRef;
        fn SecTrustCreateWithCertificates(certs: CfRef, policies: CfRef, trust: *mut CfRef) -> i32;
        fn SecTrustSetNetworkFetchAllowed(trust: CfRef, allowed: u8) -> i32;
        fn SecTrustSetVerifyDate(trust: CfRef, date: CfRef) -> i32;
        fn SecTrustSetOCSPResponse(trust: CfRef, response: CfRef) -> i32;
        fn SecTrustSetAnchorCertificates(trust: CfRef, anchors: CfRef) -> i32;
        fn SecTrustSetAnchorCertificatesOnly(trust: CfRef, only: u8) -> i32;
        fn SecTrustEvaluateWithError(trust: CfRef, error: *mut CfRef) -> u8;
    }
    const UTF8: u32 = 0x0800_0100;

    /// the anchors of an evaluation: the system's and these beside them (SecTrustSetAnchorCertificates,
    /// SecTrustSetAnchorCertificatesOnly false), or only these
    pub enum Anchors<'a> {
        System(&'a [&'a [u8]]),
        Only(&'a [&'a [u8]]),
    }

    // CFAbsoluteTime counts from 2001-01-01
    const SINCE_1970: f64 = 978_307_200.0;

    /// a CFArray of SecCertificates (None: one is no certificate the framework reads)
    unsafe fn certificates(ders: &[&[u8]]) -> Option<CfRef> {
        // SAFETY: CoreFoundation objects made here and released once each; the array retains
        // the certificates it holds.
        unsafe {
            let mut certs: Vec<CfRef> = Vec::new();
            for der in ders {
                let data = CFDataCreate(core::ptr::null(), der.as_ptr(), der.len() as isize);
                let cert = if data.is_null() { core::ptr::null() } else { SecCertificateCreateWithData(core::ptr::null(), data) };
                if !data.is_null() {
                    CFRelease(data);
                }
                if cert.is_null() {
                    certs.iter().for_each(|c| CFRelease(*c));
                    return None;
                }
                certs.push(cert);
            }
            let array = CFArrayCreate(core::ptr::null(), certs.as_ptr(), certs.len() as isize, &kCFTypeArrayCallBacks);
            certs.iter().for_each(|c| CFRelease(*c));
            (!array.is_null()).then_some(array)
        }
    }

    /// the refusal of SecTrustEvaluateWithError's error (its code an OSStatus)
    unsafe fn refused(err: CfRef) -> Refused {
        if err.is_null() {
            return Refused::new(Reason::Other, "the certificate is not trusted");
        }
        // SAFETY: the caller's error, released here; the description likewise.
        unsafe {
            let code = CFErrorGetCode(err);
            let mut text = String::from("the certificate is not trusted");
            let d = CFErrorCopyDescription(err);
            if !d.is_null() {
                let mut buf = [0u8; 512];
                if CFStringGetCString(d, buf.as_mut_ptr(), buf.len() as isize, UTF8) != 0 {
                    let n = buf.iter().position(|&c| c == 0).unwrap_or(0);
                    text = String::from_utf8_lossy(&buf[..n]).into_owned();
                }
                CFRelease(d);
            }
            CFRelease(err);
            let reason = match code {
                // errSecCertificateExpired, errSecCertificateNotValidYet
                -67818 | -67819 => Reason::Expired,
                // errSecCertificateRevoked
                -67820 => Reason::Revoked,
                // errSecHostNameMismatch
                -67602 => Reason::Name,
                // errSecNotTrusted, errSecCreateChainFailed (no path to an anchor: what macOS 15
                // says of a CA it does not know)
                -67843 | -25318 => Reason::UnknownCa,
                // errSecInvalidExtendedKeyUsage
                -67609 => Reason::Usage,
                _ => Reason::Other,
            };
            Refused::new(reason, alloc::format!("{} (Security {})", text, code))
        }
    }

    pub fn verify(chain: &[&[u8]], host: &str, ocsp: Option<&[u8]>, anchors: Anchors, time: i64) -> Result<(), Refused> {
        if chain.is_empty() {
            return Err(Refused::new(Reason::Other, "no certificate"));
        }
        let host = bare(host);
        // SAFETY: CoreFoundation objects made here and released once each; the byte slices live
        // during the calls that copy them.
        unsafe {
            let Some(array) = certificates(chain) else { return Err(Refused::new(Reason::Other, "a certificate cannot be read (Security)")) };
            let name = if host.is_empty() { core::ptr::null() } else { CFStringCreateWithBytes(core::ptr::null(), host.as_ptr(), host.len() as isize, UTF8, 0) };
            let policy = SecPolicyCreateSSL(1, name);
            if !name.is_null() {
                CFRelease(name);
            }
            let mut trust: CfRef = core::ptr::null();
            let status = SecTrustCreateWithCertificates(array, policy, &mut trust);
            CFRelease(array);
            if !policy.is_null() {
                CFRelease(policy);
            }
            if status != 0 || trust.is_null() {
                return Err(Refused::new(Reason::Other, alloc::format!("SecTrustCreateWithCertificates: {}", status)));
            }
            SecTrustSetNetworkFetchAllowed(trust, 0);
            let date = CFDateCreate(core::ptr::null(), time as f64 - SINCE_1970);
            if !date.is_null() {
                SecTrustSetVerifyDate(trust, date);
                CFRelease(date);
            }
            if let Some(r) = ocsp.filter(|r| !r.is_empty()) {
                let data = CFDataCreate(core::ptr::null(), r.as_ptr(), r.len() as isize);
                if !data.is_null() {
                    SecTrustSetOCSPResponse(trust, data);
                    CFRelease(data);
                }
            }
            // the extra CAs beside the system's (its policy and distrust hold for them too), or
            // only the roots given
            let (given, only) = match anchors {
                Anchors::System(e) => (e, false),
                Anchors::Only(r) => (r, true),
            };
            if only || !given.is_empty() {
                let Some(array) = certificates(given) else {
                    CFRelease(trust);
                    return Err(Refused::new(Reason::Other, "an anchor cannot be read (Security)"));
                };
                SecTrustSetAnchorCertificates(trust, array);
                SecTrustSetAnchorCertificatesOnly(trust, only as u8);
                CFRelease(array);
            }
            let mut err: CfRef = core::ptr::null();
            let ok = SecTrustEvaluateWithError(trust, &mut err) != 0;
            CFRelease(trust);
            if ok {
                if !err.is_null() {
                    CFRelease(err);
                }
                return Ok(());
            }
            Err(refused(err))
        }
    }
}
