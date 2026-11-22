// ct_check probe -- runs one secret-dependent operation of lib/crypto under ptrace and compares
// the instructions it executes for different secrets (tools/ct_check.py builds and drives it).
//
//   probe --list                      the names of the targets
//   probe TARGET [SEEDS]              trace TARGET with SEEDS different secrets (default 3);
//                                     prints "same STOPS MODE" or
//                                     "diff INDEX STOPS_A STOPS_B BRANCH A B CALLER MODE" (BRANCH:
//                                     the instruction where the runs part, A / B: the next one of
//                                     the first and of the differing run, CALLER: the probe's own
//                                     last addresses before it, newest first, comma-separated; as
//                                     exe+OFFSET, /LIB+OFFSET or -)
//   probe --run TARGET SEED           run once without tracing (timing, debugging)
//   probe --memcheck TARGET SEED      under valgrind --tool=memcheck: the secret marked undefined
//                                     (see memcheck below)
//   wasm32 (a cdylib): ct_run(INDEX, SEED) prepares TARGETS[INDEX] and runs it between two
//                                     calls of ct_mark; ct_check.py --wasm traces it in WABT's
//                                     wasm-interp and compares the wasm instructions and memory
//                                     addresses between the marks
//
//   probe --child TARGET SEED         one secret's run, started by the tracing probe (below)
//
// A child process is started per secret, the probe anew (fork, then execv of --child), without
// address randomization: so every secret's run begins with the same stack and heap, whatever the
// tracing probe did before.  It prepares the inputs, stops itself (SIGSTOP), runs the operation
// and stops again; the parent steps it in between and records where it goes.  MODE
// block (PTRACE_SINGLEBLOCK, the processor's branch trap; x86-64): the target of every taken
// branch; step (PTRACE_SINGLESTEP, CT_STEP=step, on ARM64, or where the branch trap is missing, as
// in some virtual machines): every instruction, some thousand times slower.  Equal sequences for all secrets mean
// control flow without secret-dependent branches or jump tables.  Not seen: memory addresses that
// depend on a secret without a branch, and instructions whose time depends on their operands
// (division, some multipliers).
//
// Linux on x86-64 (the program counter through PTRACE_PEEKUSER of rip) and ARM64 (through
// PTRACE_GETREGSET of NT_PRSTATUS; no branch trap there, so step).  The operations' inputs are
// valid, so the checks at their end (an AEAD's tag, a point at infinity) take the same way for
// every secret.

use std::hint::black_box;

unsafe extern "C" {
    fn fork() -> i32;
    fn execv(path: *const std::ffi::c_char, argv: *const *const std::ffi::c_char) -> i32;
    fn personality(persona: u64) -> i32;
    fn ptrace(request: i32, ...) -> i64;
    fn waitpid(pid: i32, status: *mut i32, options: i32) -> i32;
    fn raise(sig: i32) -> i32;
    fn kill(pid: i32, sig: i32) -> i32;
    fn _exit(code: i32) -> !;
}

const PTRACE_TRACEME: i32 = 0;
const PTRACE_CONT: i32 = 7;
const PTRACE_SINGLESTEP: i32 = 9;
/// personality(2): no address randomization (setarch -R)
const ADDR_NO_RANDOMIZE: i32 = 0x0040000;
/// stop after the next taken branch (x86: the processor's branch trap flag; ARM64 has none)
const PTRACE_SINGLEBLOCK: i32 = 33;
const SIGKILL: i32 = 9;
const SIGTRAP: i32 = 5;
const SIGSTOP: i32 = 19;

/// the traced child's program counter
#[cfg(target_arch = "x86_64")]
unsafe fn pc_of(pid: i32) -> Option<u64> {
    const PTRACE_PEEKUSER: i32 = 3;
    /// offsetof(struct user_regs_struct, rip)
    const RIP: usize = 16 * 8;
    // SAFETY: PEEKUSER reads one word of the stopped child's registers
    let pc = unsafe { ptrace(PTRACE_PEEKUSER, pid, RIP, 0usize) };
    // -1 is the failure (errno), never a user address
    (pc != -1).then_some(pc as u64)
}

/// no tracer on other processors (wasm32: ct_run below instead)
#[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
unsafe fn pc_of(_pid: i32) -> Option<u64> {
    None
}

/// the traced child's program counter: struct user_pt_regs { regs[31], sp, pc, pstate }
#[cfg(target_arch = "aarch64")]
unsafe fn pc_of(pid: i32) -> Option<u64> {
    const PTRACE_GETREGSET: i32 = 0x4204;
    const NT_PRSTATUS: usize = 1;
    #[repr(C)]
    struct Iovec {
        base: *mut u64,
        len: usize,
    }
    let mut regs = [0u64; 34];
    let mut iov = Iovec { base: regs.as_mut_ptr(), len: core::mem::size_of_val(&regs) };
    // SAFETY: GETREGSET writes at most iov.len bytes of the stopped child's registers into regs
    let r = unsafe { ptrace(PTRACE_GETREGSET, pid, NT_PRSTATUS, &mut iov as *mut Iovec) };
    // the kernel sets iov.len to what it wrote: the whole set, or pc is not among it
    (r == 0 && iov.len == core::mem::size_of_val(&regs)).then_some(regs[32])
}
/// the most instructions one operation may take
const MAX_STEPS: usize = 200_000_000;
/// the most instructions followed one by one in the block where two runs part
const FINE_MAX: usize = 2_000_000;

// ------------------------------------------------------------------ secrets

/// deterministic bytes of one secret (xorshift64*; seed 0 is not used)
fn secret_bytes(seed: u64, n: usize) -> Vec<u8> {
    let mut s = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    (0..n)
        .map(|_| {
            s ^= s >> 12;
            s ^= s << 25;
            s ^= s >> 27;
            (s.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 56) as u8
        })
        .collect()
}

fn a32(b: &[u8]) -> [u8; 32] {
    b[..32].try_into().unwrap()
}

/// a P-256 scalar in [1, n - 1] from the secret's bytes (drawn again from the next 32 bytes)
fn p256_scalar(b: &[u8]) -> [u8; 32] {
    b.chunks_exact(32).find_map(|c| crypto::p256::secret_from(&a32(c))).expect("a scalar")
}

const MSG: &[u8] = b"a message of fixed length that is signed, sealed and hashed by the probe ..";

#[inline(never)]
fn leak_tick(x: u64) -> u64 {
    black_box(x).wrapping_mul(3).wrapping_add(1)
}

// ------------------------------------------------------------------ targets

type Op = Box<dyn FnMut()>;

const TARGETS: &[&str] = &[
    "selftest_leak",
    "selftest_lookup",
    "ct_eq",
    "x25519",
    "x25519_shared",
    "ed25519_keypair",
    "ed25519_sign",
    "p256_public_key",
    "p256_shared",
    "p256_mul_point",
    "eappwd_pwe",
    "eappwd_commit",
    "eappwd_shared",
    "bn_pow_ct",
    "chacha20poly1305_seal",
    "chacha20poly1305_open",
    "xchacha20poly1305_seal",
    "xchacha20poly1305_open",
    "aes128gcm_seal",
    "aes256gcm_open",
    "aes_keywrap",
    "aes_cmac",
    "poly1305",
    "sealbox_open",
    "hmac_sha256",
    "hmac_sha512",
    "hkdf_sha256",
    "pbkdf2_sha256",
    "hmac_sha1",
    "hmac_md5",
    "sha256",
    "sha512",
    "blake2b_keyed",
    "blake3_keyed",
    "argon2id",
    "b64_encode",
    "b64_decode",
    "hex",
    "unhex",
    #[cfg(feature = "netio")]
    "pem_pkcs8_key",
    #[cfg(feature = "netio")]
    "ssh_private_key",
];

/// the operation of a target with the inputs of one secret, prepared outside the trace
fn prepare(name: &str, seed: u64) -> Op {
    prepare_from(name, secret_bytes(seed, 4096))
}

/// the operation of a target with the inputs made from the secret bytes `s` (4096 of them)
fn prepare_from(name: &str, s: Vec<u8>) -> Op {
    use crypto::*;
    let k = a32(&s);
    let nonce12 = [7u8; 12];
    let nonce24 = [7u8; 24];
    let plain: Vec<u8> = s[64..64 + MSG.len()].to_vec();
    match name {
        // a deliberate leak: a call for every set bit of the secret (a call is no select the
        // optimizer could make of the branch; the tracer must see it)
        "selftest_leak" => Box::new(move || {
            let mut acc = 0u64;
            for i in 0..64 {
                if (k[i / 8] >> (i % 8)) & 1 == 1 {
                    acc = leak_tick(acc);
                }
            }
            black_box(acc);
        }),
        // a deliberate table lookup at a secret index (memcheck must see it; the trace does not)
        "selftest_lookup" => {
            let table: Vec<u64> = (0..256u64).collect();
            Box::new(move || {
                black_box(table[k[3] as usize]);
            })
        }
        "ct_eq" => {
            let (a, b) = (s[..32].to_vec(), s[32..64].to_vec());
            Box::new(move || {
                black_box(ct_eq(black_box(&a), black_box(&b)));
            })
        }
        "x25519" => Box::new(move || {
            black_box(x25519::x25519(black_box(&k), black_box(&[9u8; 32])));
        }),
        "x25519_shared" => {
            let peer = x25519::public_key(&[0x42; 32]);
            Box::new(move || {
                black_box(x25519::shared(black_box(&k), black_box(&peer)));
            })
        }
        "ed25519_keypair" => Box::new(move || {
            black_box(ed25519::Keypair::from_seed(black_box(&k)).public);
        }),
        "ed25519_sign" => {
            let kp = ed25519::Keypair::from_seed(&k);
            Box::new(move || {
                black_box(kp.sign(black_box(MSG)));
            })
        }
        "p256_public_key" => {
            let sc = p256_scalar(&s);
            Box::new(move || {
                black_box(p256::public_key(black_box(&sc)));
            })
        }
        "p256_shared" => {
            let sc = p256_scalar(&s);
            let peer = p256::public_key(&p256_scalar(&[0x42; 64]));
            Box::new(move || {
                black_box(p256::shared(black_box(&sc), black_box(&peer)));
            })
        }
        "p256_mul_point" => {
            let sc = p256_scalar(&s);
            let pk = p256::public_key(&p256_scalar(&[0x43; 64]));
            let pt: [u8; 64] = pk[1..].try_into().unwrap();
            Box::new(move || {
                black_box(p256::mul_point(black_box(&sc), black_box(&pt)));
            })
        }
        // the password is the secret (always 16 bytes: its length is public)
        "eappwd_pwe" => {
            let pw = s[..16].to_vec();
            Box::new(move || {
                black_box(eappwd::pwe(&[1, 2, 3, 4], b"peer@example.org", b"server", black_box(&pw)));
            })
        }
        "eappwd_commit" => {
            let pwe = eappwd::pwe(&[1, 2, 3, 4], b"peer@example.org", b"server", &s[..16]);
            let (rand, mask) = (p256_scalar(&s[64..]), p256_scalar(&s[1024..]));
            Box::new(move || {
                black_box(eappwd::commit(black_box(&pwe), black_box(&rand), black_box(&mask)));
            })
        }
        "eappwd_shared" => {
            let pwe = eappwd::pwe(&[1, 2, 3, 4], b"peer@example.org", b"server", &s[..16]);
            let rand = p256_scalar(&s[64..]);
            let other = eappwd::pwe(&[1, 2, 3, 4], b"peer@example.org", b"server", b"other password..");
            let (ps, pe) = eappwd::commit(&other, &p256_scalar(&[0x44; 64]), &p256_scalar(&[0x45; 64])).expect("commit");
            Box::new(move || {
                black_box(eappwd::shared(black_box(&pwe), black_box(&rand), black_box(&ps), black_box(&pe)));
            })
        }
        // a 1024-bit odd modulus, base 2, a 512-bit secret exponent (RSA signing, the DH of plat's
        // Secret Service client)
        "bn_pow_ct" => {
            let mut nb = [0xC3u8; 128];
            nb[127] |= 1;
            let m = bn::Mont::new(&bn::from_be(&nb, 0)).expect("modulus");
            let e = s[..64].to_vec();
            Box::new(move || {
                black_box(m.pow_ct(black_box(&[2]), black_box(&e)));
            })
        }
        "chacha20poly1305_seal" => Box::new(move || {
            black_box(aead::seal(black_box(&k), &nonce12, b"aad", black_box(&plain)));
        }),
        "chacha20poly1305_open" => {
            let sealed = aead::seal(&k, &nonce12, b"aad", &plain);
            Box::new(move || {
                black_box(aead::open(black_box(&k), &nonce12, b"aad", black_box(&sealed)));
            })
        }
        "xchacha20poly1305_seal" => Box::new(move || {
            black_box(aead::x_seal(black_box(&k), &nonce24, b"aad", black_box(&plain)));
        }),
        "xchacha20poly1305_open" => {
            let sealed = aead::x_seal(&k, &nonce24, b"aad", &plain);
            Box::new(move || {
                black_box(aead::x_open(black_box(&k), &nonce24, b"aad", black_box(&sealed)));
            })
        }
        "aes128gcm_seal" => Box::new(move || {
            let g = gcm::AesGcm::new(black_box(&k[..16])).expect("key");
            let mut buf = plain.clone();
            black_box(g.seal_in_place(&nonce12, b"aad", black_box(&mut buf)));
            black_box(buf);
        }),
        "aes256gcm_open" => {
            let g = gcm::AesGcm::new(&k).expect("key");
            let mut sealed = plain.clone();
            let tag = g.seal_in_place(&nonce12, b"aad", &mut sealed);
            Box::new(move || {
                let g = gcm::AesGcm::new(black_box(&k)).expect("key");
                let mut buf = sealed.clone();
                black_box(g.open_in_place(&nonce12, b"aad", black_box(&mut buf), &tag));
                black_box(buf);
            })
        }
        "aes_keywrap" => {
            let wrapped = aes_wrap(&k[..16], &s[32..64]).expect("wrap");
            Box::new(move || {
                black_box(aes_unwrap(black_box(&k[..16]), black_box(&wrapped)));
            })
        }
        "aes_cmac" => Box::new(move || {
            black_box(cmac_aes(black_box(&k[..16]), black_box(&plain)));
        }),
        "poly1305" => Box::new(move || {
            black_box(poly1305::poly1305(black_box(&k), black_box(&plain)));
        }),
        "sealbox_open" => {
            let sealed = sealbox::seal(&x25519::public_key(&k), &[0x46; 56], b"aad", &plain).expect("seal");
            Box::new(move || {
                black_box(sealbox::open(black_box(&k), b"aad", black_box(&sealed)));
            })
        }
        "hmac_sha256" => Box::new(move || {
            black_box(hmac_sha256(black_box(&k), black_box(&plain)));
        }),
        "hmac_sha512" => Box::new(move || {
            black_box(hmac_sha512(black_box(&k), black_box(&plain)));
        }),
        "hkdf_sha256" => Box::new(move || {
            let mut out = [0u8; 64];
            hkdf_sha256(b"salt", black_box(&k), b"info", &mut out);
            black_box(out);
        }),
        "pbkdf2_sha256" => Box::new(move || {
            let mut out = [0u8; 32];
            pbkdf2_sha256(black_box(&k[..16]), b"salt", 2, &mut out);
            black_box(out);
        }),
        "hmac_sha1" => Box::new(move || {
            black_box(hmac_sha1(black_box(&k), black_box(&plain)));
        }),
        "hmac_md5" => Box::new(move || {
            black_box(hmac_md5(black_box(&k), black_box(&plain)));
        }),
        "sha256" => Box::new(move || {
            black_box(sha256(black_box(&plain)));
        }),
        "sha512" => Box::new(move || {
            black_box(sha512(black_box(&plain)));
        }),
        "blake2b_keyed" => Box::new(move || {
            let mut out = [0u8; 32];
            blake2b::blake2b(&mut out, black_box(&k), &[black_box(&plain)]);
            black_box(out);
        }),
        "blake3_keyed" => Box::new(move || {
            black_box(blake3::keyed_hash(black_box(&k), black_box(&plain)));
        }),
        // small parameters: the same code as the real ones, fewer blocks
        "argon2id" => {
            let pw = s[..16].to_vec();
            Box::new(move || {
                let p = argon2::Params { m_kib: 32, t: 1, p: 1, out_len: 32 };
                let mut out = [0u8; 32];
                black_box(argon2::argon2id(black_box(&pw), b"saltsaltsalt", &p, &mut out));
                black_box(out);
            })
        }
        // keys as text: PEM, OpenSSH key files, PHC strings, hex keys (the encoded strings of
        // different secrets have the same length)
        "b64_encode" => Box::new(move || {
            black_box(b64::encode(black_box(&s[..48])));
        }),
        "b64_decode" => {
            let mut text = b64::encode(&s[..47]);
            public_layout(&mut text);
            Box::new(move || {
                black_box(b64::decode(black_box(&text)));
            })
        }
        "hex" => Box::new(move || {
            black_box(hex(black_box(&k)));
        }),
        "unhex" => {
            let mut text = hex(&k);
            public_layout(&mut text);
            Box::new(move || {
                black_box(unhex(black_box(&text)));
            })
        }
        // a private key from its PEM file (PKCS #8, as BackupCraft's keys)
        #[cfg(feature = "netio")]
        "pem_pkcs8_key" => {
            let mut text = netio::pem::encode("PRIVATE KEY", &netio::x509::ed25519_pkcs8(&k));
            public_layout(&mut text);
            Box::new(move || {
                let der = netio::pem::decode(black_box(&text), "PRIVATE KEY").expect("pem");
                black_box(netio::x509::ed25519_from_pkcs8(&der));
            })
        }
        // an OpenSSH key file (netio's SSH client logs in with it)
        #[cfg(feature = "netio")]
        "ssh_private_key" => {
            let mut text = netpkt::ssh::KeyPair::ed25519(&k).private_file("probe");
            public_layout(&mut text);
            Box::new(move || {
                black_box(netio::ssh::private_key(black_box(&text)).map(|kp| kp.public));
            })
        }
        _ => {
            eprintln!("unknown target {}", name);
            std::process::exit(2);
        }
    }
}

// ------------------------------------------------------------------ valgrind

/// a valgrind client request (x86-64: the magic preamble, rax -> the arguments, rdx -> the result;
/// natively a no-op that gives `default`)
#[cfg(target_arch = "x86_64")]
fn vg_request(default: usize, args: [usize; 6]) -> usize {
    let mut result = default;
    // SAFETY: the rotations leave rdi as it was (3 + 13 + 61 + 51 = 128), xchg rbx, rbx changes
    // nothing; valgrind reads the six words at rax and writes the answer into rdx
    unsafe {
        core::arch::asm!(
            "rol rdi, 3", "rol rdi, 13", "rol rdi, 61", "rol rdi, 51", "xchg rbx, rbx",
            inout("rdx") result,
            in("rax") args.as_ptr(),
            inout("rdi") 0usize => _,
            options(nostack),
        );
    }
    result
}

/// a valgrind client request (ARM64, valgrind.h: the preamble rotates x12 by 128 bits, x4 -> the
/// arguments, x3 -> the default and the result, orr x10, x10, x10 marks the request; natively a
/// no-op that gives `default`)
#[cfg(target_arch = "aarch64")]
fn vg_request(default: usize, args: [usize; 6]) -> usize {
    let mut result = default;
    // SAFETY: the rotations leave x12 as it was (3 + 13 + 51 + 61 = 128), orr x10, x10, x10
    // changes nothing; valgrind reads the six words at x4 and writes the answer into x3
    unsafe {
        core::arch::asm!(
            "ror x12, x12, #3", "ror x12, x12, #13", "ror x12, x12, #51", "ror x12, x12, #61", "orr x10, x10, x10",
            inout("x3") result,
            in("x4") args.as_ptr(),
            inout("x12") 0usize => _,
            options(nostack),
        );
    }
    result
}

/// no valgrind on other processors
#[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
fn vg_request(default: usize, _args: [usize; 6]) -> usize {
    default
}

const VG_RUNNING_ON_VALGRIND: usize = 0x1001;
const VG_CHANGE_ERR_DISABLEMENT: usize = 0x1801;
/// VG_USERREQ_TOOL_BASE('M', 'C') + 1, + 2
const VG_MAKE_MEM_UNDEFINED: usize = 0x4d43_0001;
const VG_MAKE_MEM_DEFINED: usize = 0x4d43_0002;

/// --memcheck TARGET SEED (under valgrind's memcheck): the secret bytes are marked undefined, so
/// memcheck follows everything made from them and reports a branch on them ("Conditional jump")
/// or an address made from them ("Use of uninitialised value"); reporting is off while the
/// inputs are prepared and on only for the operation
fn memcheck(name: &str, seed: u64) -> i32 {
    if vg_request(0, [VG_RUNNING_ON_VALGRIND, 0, 0, 0, 0, 0]) == 0 {
        eprintln!("--memcheck runs under valgrind --tool=memcheck only");
        return 2;
    }
    // where valgrind loaded this program (its own choice, which differs between architectures):
    // ct_check.py looks the frames of memcheck's reports up with addr2line from there
    let maps = Maps::read();
    let own: Vec<&(u64, u64, String)> = maps.ranges.iter().filter(|r| std::path::Path::new(&r.2) == maps.exe).collect();
    if let (Some(lo), Some(hi)) = (own.iter().map(|r| r.0).min(), own.iter().map(|r| r.1).max()) {
        println!("exe {:#x} {:#x}", lo, hi);
    }
    let s = secret_bytes(seed, 4096);
    vg_request(0, [VG_CHANGE_ERR_DISABLEMENT, 1, 0, 0, 0, 0]);
    vg_request(0, [VG_MAKE_MEM_UNDEFINED, s.as_ptr() as usize, s.len(), 0, 0, 0]);
    let mut op = prepare_from(name, s);
    vg_request(0, [VG_CHANGE_ERR_DISABLEMENT, usize::MAX, 0, 0, 0, 0]); // -1: on again
    probe_op(&mut op);
    vg_request(0, [VG_CHANGE_ERR_DISABLEMENT, 1, 0, 0, 0, 0]);
    0
}

/// a text made from the secret keeps it in its characters, while its length and capacity are
/// public (a file's size): under memcheck they are marked defined (String::push adds each
/// character's UTF-8 length, which memcheck would otherwise follow from the secret)
fn public_layout(text: &mut String) {
    let at = text as *mut String as usize;
    vg_request(0, [VG_MAKE_MEM_DEFINED, at, core::mem::size_of::<String>(), 0, 0, 0]);
}

/// the operation itself, in a frame of its own (memcheck's reports name it)
#[inline(never)]
fn probe_op(op: &mut Op) {
    op();
}

// ------------------------------------------------------------------ tracing

/// the mappings of this process (the child is a fork of it, so they are its mappings too)
struct Maps {
    exe: std::path::PathBuf,
    /// start, end, file
    ranges: Vec<(u64, u64, String)>,
}

impl Maps {
    fn read() -> Maps {
        let exe = std::fs::read_link("/proc/self/exe").unwrap_or_default();
        let text = std::fs::read_to_string("/proc/self/maps").unwrap_or_default();
        let mut ranges = Vec::new();
        for ln in text.lines() {
            let f: Vec<&str> = ln.split_whitespace().collect();
            let Some(path) = f.get(5) else { continue };
            let mut r = f[0].split('-').map(|x| u64::from_str_radix(x, 16).unwrap_or(0));
            ranges.push((r.next().unwrap_or(0), r.next().unwrap_or(0), String::from(*path)));
        }
        Maps { exe, ranges }
    }

    fn file(&self, addr: u64) -> Option<&str> {
        self.ranges.iter().find(|r| (r.0..r.1).contains(&addr)).map(|r| r.2.as_str())
    }

    fn in_exe(&self, addr: u64) -> bool {
        self.file(addr).is_some_and(|f| std::path::Path::new(f) == self.exe)
    }

    /// "exe+0x..." (an offset addr2line takes), "/path/of/lib.so+0x..." (from its load address)
    /// or "0x..."; "-" for none
    fn locate(&self, addr: Option<u64>) -> String {
        let Some(addr) = addr else { return String::from("-") };
        let Some(path) = self.file(addr) else { return format!("{:#x}", addr) };
        // the lowest mapping of a file is its load address
        let base = self.ranges.iter().filter(|r| r.2 == path).map(|r| r.0).min().unwrap_or(0);
        let name = if std::path::Path::new(path) == self.exe { "exe" } else { path };
        format!("{}+{:#x}", name, addr - base)
    }
}

/// an execv of the probe itself: its path, the arguments (argv[0] first) and the pointers to them
/// that execv takes, ending in null (they point into `_args`, which is kept for them)
struct Exec {
    path: std::ffi::CString,
    _args: Vec<std::ffi::CString>,
    argv: Vec<*const std::ffi::c_char>,
}

impl Exec {
    fn new(args: impl IntoIterator<Item = Vec<u8>>) -> Result<Exec, String> {
        let path = std::env::current_exe().map_err(|e| format!("the probe's own path: {}", e))?;
        let path = std::ffi::CString::new(path.into_os_string().into_encoded_bytes()).map_err(|_| "a NUL in the probe's path")?;
        let args = args.into_iter().map(std::ffi::CString::new).collect::<Result<Vec<_>, _>>().map_err(|_| "a NUL in an argument")?;
        let mut argv: Vec<*const std::ffi::c_char> = args.iter().map(|a| a.as_ptr()).collect();
        argv.push(std::ptr::null());
        Ok(Exec { path, _args: args, argv })
    }

    /// returns only when execv fails
    fn run(&self) {
        // SAFETY: a path and a null-terminated argument list of C strings, alive for the call
        unsafe {
            execv(self.path.as_ptr(), self.argv.as_ptr());
        }
    }
}

/// the addresses the operation of `name` reaches with the secret of `seed`: the target of every
/// taken branch (PTRACE_SINGLEBLOCK) or every instruction (PTRACE_SINGLESTEP); with `fine_from`,
/// after that many entries every instruction, at most FINE_MAX more
fn trace(name: &str, seed: u64, step: i32, fine_from: usize) -> Result<Vec<u64>, String> {
    // The child is the probe started anew (--child), not a fork that goes on: a fork takes the
    // parent's stack and heap over, and those differ between the secrets (the depth of the call
    // that got here, the traces allocated before), while memset's and memcpy's ways follow the
    // alignment of the addresses they get -- at opt-level z on ARM64, HMAC-SHA1's buffers on the
    // stack lay 112 bytes deeper for the second and third secret, and memset took two rounds
    // more.  Started anew, without address randomization (main), every secret's run begins with
    // the same layout; the seed has a fixed width, so the arguments on the new stack are as long.
    let exec = Exec::new([b"probe".to_vec(), b"--child".to_vec(), name.as_bytes().to_vec(), format!("{:020}", seed).into_bytes()])?;
    // SAFETY: fork, ptrace, execv, waitpid and kill are called as their manual pages describe;
    // the child only asks to be traced and becomes the probe (--child), or ends with _exit.
    unsafe {
        let pid = fork();
        if pid < 0 {
            return Err("fork failed".into());
        }
        if pid == 0 {
            ptrace(PTRACE_TRACEME, 0i32, 0usize, 0usize);
            exec.run();
            _exit(127);
        }
        let mut st = 0i32;
        let end = |pid: i32, st: &mut i32| {
            kill(pid, SIGKILL);
            waitpid(pid, st, 0);
        };
        // its stop at execv (a SIGTRAP, PTRACE_TRACEME), then on to its own before the operation
        if waitpid(pid, &mut st, 0) != pid || st & 0xff != 0x7f || (st >> 8) & 0xff != SIGTRAP {
            end(pid, &mut st);
            return Err(format!("the child did not start (status {:#x})", st));
        }
        if ptrace(PTRACE_CONT, pid, 0usize, 0usize) < 0 || waitpid(pid, &mut st, 0) != pid || st & 0xff != 0x7f || (st >> 8) & 0xff != SIGSTOP {
            end(pid, &mut st);
            return Err(format!("the child did not stop before the operation (status {:#x})", st));
        }
        let mut out = Vec::with_capacity(1 << 20);
        loop {
            let req = if out.len() >= fine_from { PTRACE_SINGLESTEP } else { step };
            if ptrace(req, pid, 0usize, 0usize) < 0 {
                end(pid, &mut st);
                return Err(if req == PTRACE_SINGLEBLOCK { "no-block".into() } else { "PTRACE_SINGLESTEP failed".into() });
            }
            if waitpid(pid, &mut st, 0) != pid || st & 0xff != 0x7f {
                return Err(format!("the child ended during the operation (status {:#x})", st));
            }
            match (st >> 8) & 0xff {
                SIGTRAP => match pc_of(pid) {
                    Some(pc) => out.push(pc),
                    None => {
                        end(pid, &mut st);
                        return Err("the child's program counter could not be read".into());
                    }
                },
                SIGSTOP => break,
                sig => {
                    end(pid, &mut st);
                    return Err(format!("signal {} during the operation", sig));
                }
            }
            if out.len() > MAX_STEPS || out.len() >= fine_from.saturating_add(FINE_MAX) {
                break;
            }
        }
        end(pid, &mut st);
        if out.len() > MAX_STEPS {
            return Err(format!("more than {} steps", MAX_STEPS));
        }
        Ok(out)
    }
}

/// where two runs part: the last address both reached (the branch), the next of each, and the
/// probe's own last addresses before it (newest first; when the branch lies in a library, the
/// PLT stub and the call that led there)
fn parting(name: &str, step: i32, seed: u64, a: &[u64], b: &[u64]) -> Result<(Option<u64>, Option<u64>, Option<u64>, Vec<u64>), String> {
    let i = a.iter().zip(b).position(|(x, y)| x != y).unwrap_or(a.len().min(b.len()));
    let (common, na, nb) = if step == PTRACE_SINGLEBLOCK {
        // the block where they part, instruction by instruction
        let fa = trace(name, 1, step, i)?;
        let fb = trace(name, seed, step, i)?;
        let j = fa.iter().zip(&fb).position(|(x, y)| x != y).unwrap_or(fa.len().min(fb.len()));
        (fa[..j].to_vec(), fa.get(j).copied(), fb.get(j).copied())
    } else {
        (a[..i].to_vec(), a.get(i).copied(), b.get(i).copied())
    };
    let maps = Maps::read();
    let mut k = common.len();
    while k > 0 && !maps.in_exe(common[k - 1]) {
        k -= 1;
    }
    let mut callers = Vec::new();
    while k > 0 && callers.len() < 4 && maps.in_exe(common[k - 1]) {
        callers.push(common[k - 1]);
        k -= 1;
    }
    Ok((common.last().copied(), na, nb, callers))
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("--list") => {
            for t in TARGETS {
                println!("{}", t);
            }
        }
        Some("--memcheck") => {
            let name = args.get(2).map(String::as_str).unwrap_or("");
            let seed: u64 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(1);
            std::process::exit(memcheck(name, seed));
        }
        Some("--run") => {
            let name = args.get(2).map(String::as_str).unwrap_or("");
            let seed: u64 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(1);
            let mut op = prepare(name, seed);
            let t = std::time::Instant::now();
            op();
            println!("{} us", t.elapsed().as_micros());
        }
        Some("--child") => {
            // one secret's run, started by trace: prepare, stop, the operation, stop
            let name = args.get(2).map(String::as_str).unwrap_or("");
            let seed: u64 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(1);
            let mut op = prepare(name, seed);
            // SAFETY: raise and _exit as their manual pages describe
            unsafe {
                raise(SIGSTOP);
                op();
                raise(SIGSTOP);
                _exit(0);
            }
        }
        Some(name) => {
            // without address randomization (what setarch -R does), started again so: the children
            // then have the layout of this process, whose /proc/self/maps names their addresses,
            // and the same in every one
            // SAFETY: personality as its manual page describes
            unsafe {
                let p = personality(0xffff_ffff);
                if p != -1 && p & ADDR_NO_RANDOMIZE == 0 && personality((p | ADDR_NO_RANDOMIZE) as u64) != -1 {
                    if let Ok(e) = Exec::new(std::env::args_os().map(|a| a.into_encoded_bytes())) {
                        e.run();
                    }
                    // no new start: the children as randomized as this process
                    personality(p as u64);
                }
            }
            let seeds: u64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(3).max(2);
            let mut step = if std::env::var("CT_STEP").as_deref() == Ok("step") || cfg!(target_arch = "aarch64") { PTRACE_SINGLESTEP } else { PTRACE_SINGLEBLOCK };
            let mut first = trace(name, 1, step, usize::MAX);
            if step == PTRACE_SINGLEBLOCK && first.as_ref().is_err_and(|e| e == "no-block") {
                // a processor or virtual machine without the branch trap: every instruction
                step = PTRACE_SINGLESTEP;
                first = trace(name, 1, step, usize::MAX);
            }
            let mode = if step == PTRACE_SINGLEBLOCK { "block" } else { "step" };
            let first = match first {
                Ok(t) => t,
                Err(e) => {
                    println!("error {}", e);
                    std::process::exit(3);
                }
            };
            let traced = |seed: u64| match trace(name, seed, step, usize::MAX) {
                Ok(t) => t,
                Err(e) => {
                    println!("error {}", e);
                    std::process::exit(3);
                }
            };
            // every secret against the first: the earliest parting counts (an expected late one,
            // Argon2id's, must not hide an early one of another secret); its trace is made again
            // at the end, so at most two are held (single steps on ARM64 take hundreds of MB)
            let mut earliest: Option<(usize, u64)> = None;
            for seed in 2..=seeds {
                let other = traced(seed);
                if other != first {
                    let i = first.iter().zip(&other).position(|(a, b)| a != b).unwrap_or(first.len().min(other.len()));
                    if earliest.is_none_or(|(e, _)| i < e) {
                        earliest = Some((i, seed));
                    }
                }
            }
            let Some((i, seed)) = earliest else {
                println!("same {} {}", first.len(), mode);
                return;
            };
            let other = traced(seed);
            let maps = Maps::read();
            let at = match parting(name, step, seed, &first, &other) {
                Ok((br, na, nb, callers)) => {
                    let c: Vec<String> = callers.iter().map(|&x| maps.locate(Some(x))).collect();
                    [maps.locate(br), maps.locate(na), maps.locate(nb), if c.is_empty() { String::from("-") } else { c.join(",") }]
                }
                Err(e) => [e.replace(' ', "_"), String::from("-"), String::from("-"), String::from("-")],
            };
            println!("diff {} {} {} {} {} {} {} {}", i, first.len(), other.len(), at[0], at[1], at[2], at[3], mode);
        }
        None => {
            eprintln!("usage: probe --list | TARGET [SEEDS] | --run TARGET SEED | --memcheck TARGET SEED");
            std::process::exit(2);
        }
    }
}

// ------------------------------------------------------------------ wasm32

/// the marks around the operation: a call the trace shows (ct_check.py --wasm looks for it)
#[cfg(target_arch = "wasm32")]
#[unsafe(no_mangle)]
#[inline(never)]
pub extern "C" fn ct_mark() {
    black_box(());
}

/// TARGETS[index] with the inputs of secret `seed`, prepared before the first mark, run between
/// the marks -> 0, or 1 for an index past the list
#[cfg(target_arch = "wasm32")]
#[unsafe(no_mangle)]
pub extern "C" fn ct_run(index: u32, seed: u64) -> u32 {
    let Some(name) = TARGETS.get(index as usize) else { return 1 };
    let mut op = prepare(name, seed);
    ct_mark();
    op();
    ct_mark();
    0
}
