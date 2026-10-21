// certs -- the operating system's trusted certificate authorities
//
//   verify_tls_server(chain, host) -> Option<Result<(), String>>
//        Windows: CryptoAPI builds and checks the chain (leaf first, DER) for a TLS server of
//        that name, with the machine's and user's roots, enterprise policy and automatic root
//        updates; Some(Err(text)) when it refuses.  Linux: None (netio checks against the
//        bundle).
//   root_bundle() -> Option<Vec<u8>>
//        Linux: the PEM file of the distribution's CA bundle (Debian / Ubuntu, Fedora / RHEL,
//        openSUSE, Alpine).  Windows: None.
//
// Revocation is not checked (no network access during the handshake).

use alloc::string::String;
use alloc::vec::Vec;

#[cfg(target_os = "linux")]
pub fn root_bundle() -> Option<Vec<u8>> {
    const PATHS: [&str; 5] = [
        "/etc/ssl/certs/ca-certificates.crt",
        "/etc/pki/tls/certs/ca-bundle.crt",
        "/etc/ssl/ca-bundle.pem",
        "/etc/pki/ca-trust/extracted/pem/tls-ca-bundle.pem",
        "/etc/ssl/cert.pem",
    ];
    PATHS.iter().find_map(|p| crate::read_file_raw(p))
}

#[cfg(windows)]
pub fn root_bundle() -> Option<Vec<u8>> {
    None
}

#[cfg(target_os = "linux")]
pub fn verify_tls_server(_chain: &[&[u8]], _host: &str) -> Option<Result<(), String>> {
    None
}

#[cfg(windows)]
mod win {
    use core::ffi::c_void;

    pub const ENCODING: u32 = 0x0000_0001 | 0x0001_0000; // X509_ASN_ENCODING | PKCS_7_ASN_ENCODING
    pub const STORE_PROV_MEMORY: usize = 2;
    pub const STORE_ADD_ALWAYS: u32 = 4;
    pub const CHAIN_POLICY_SSL: usize = 4;
    pub const AUTHTYPE_SERVER: u32 = 2;

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

    #[link(name = "crypt32", kind = "raw-dylib")]
    unsafe extern "system" {
        pub fn CertCreateCertificateContext(enc: u32, der: *const u8, len: u32) -> *const c_void;
        pub fn CertFreeCertificateContext(ctx: *const c_void) -> i32;
        pub fn CertOpenStore(provider: usize, enc: u32, prov: usize, flags: u32, para: *const c_void) -> *mut c_void;
        pub fn CertCloseStore(store: *mut c_void, flags: u32) -> i32;
        pub fn CertAddEncodedCertificateToStore(store: *mut c_void, enc: u32, der: *const u8, len: u32, disposition: u32, out: *mut *const c_void) -> i32;
        pub fn CertGetCertificateChain(engine: *mut c_void, ctx: *const c_void, time: *const c_void, extra: *mut c_void, para: *const ChainPara, flags: u32, reserved: *mut c_void, chain: *mut *const c_void) -> i32;
        pub fn CertFreeCertificateChain(chain: *const c_void);
        pub fn CertVerifyCertificateChainPolicy(policy: usize, chain: *const c_void, para: *const PolicyPara, status: *mut PolicyStatus) -> i32;
    }
}

#[cfg(windows)]
pub fn verify_tls_server(chain: &[&[u8]], host: &str) -> Option<Result<(), String>> {
    use core::ptr::{null, null_mut};
    use win::*;
    let leaf = chain.first()?;
    let server_auth = b"1.3.6.1.5.5.7.3.1\0";
    let ids = [server_auth.as_ptr()];
    let name: Vec<u16> = host.encode_utf16().chain(core::iter::once(0)).collect();
    // SAFETY: every pointer handed to CryptoAPI points into a buffer that lives until the call
    // returns; contexts, stores and chains are freed on every path below.
    unsafe {
        let ctx = CertCreateCertificateContext(ENCODING, leaf.as_ptr(), leaf.len() as u32);
        if ctx.is_null() {
            return Some(Err(String::from("Zertifikat nicht lesbar")));
        }
        let store = CertOpenStore(STORE_PROV_MEMORY, 0, 0, 0, null());
        for c in &chain[1..] {
            if !store.is_null() {
                CertAddEncodedCertificateToStore(store, ENCODING, c.as_ptr(), c.len() as u32, STORE_ADD_ALWAYS, null_mut());
            }
        }
        let para = ChainPara { size: core::mem::size_of::<ChainPara>() as u32, usage: UsageMatch { ty: 0, usage: CtlUsage { count: 1, ids: ids.as_ptr() } } };
        let mut ch: *const core::ffi::c_void = null();
        let ok = CertGetCertificateChain(null_mut(), ctx, null(), store, &para, 0, null_mut(), &mut ch) != 0;
        let result = if !ok || ch.is_null() {
            Err(String::from("keine Zertifikatskette"))
        } else {
            let extra = SslExtra { size: core::mem::size_of::<SslExtra>() as u32, auth_type: AUTHTYPE_SERVER, checks: 0, server_name: if host.is_empty() { null() } else { name.as_ptr() } };
            let pp = PolicyPara { size: core::mem::size_of::<PolicyPara>() as u32, flags: 0, extra: &extra as *const SslExtra as *const core::ffi::c_void };
            let mut st = PolicyStatus { size: core::mem::size_of::<PolicyStatus>() as u32, error: 0, chain_index: 0, element_index: 0, extra: null_mut() };
            let r = CertVerifyCertificateChainPolicy(CHAIN_POLICY_SSL, ch, &pp, &mut st);
            CertFreeCertificateChain(ch);
            if r == 0 {
                Err(String::from("Prüfung nicht möglich"))
            } else {
                match st.error {
                    0 => Ok(()),
                    0x800B_0109 => Err(String::from("unbekannte Zertifizierungsstelle")),
                    0x800B_010F => Err(String::from("Zertifikat gilt nicht für diesen Namen")),
                    0x800B_0101 => Err(String::from("Zertifikat abgelaufen")),
                    0x8009_2010 => Err(String::from("Zertifikat gesperrt")),
                    e => Err(alloc::format!("Zertifikat abgelehnt (0x{:08X})", e)),
                }
            }
        };
        if !store.is_null() {
            CertCloseStore(store, 0);
        }
        CertFreeCertificateContext(ctx);
        Some(result)
    }
}
