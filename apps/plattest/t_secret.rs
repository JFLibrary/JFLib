// t_secret -- the system's secret store (lib/plat/secret.rs, win_secret.rs, linux_secret.rs)
//
//   plattest --test secret       this system's store: Windows the Credential Manager (DPAPI files
//                                below data:// for values above 2560 bytes), Linux the Secret
//                                Service of the session bus.  set, get, overwrite, a second and
//                                an empty account, binary values, names that differ in a '/',
//                                another app's prefix (set_app_id), large values up to
//                                SECRET_MAX, a changed DPAPI file, invalid names, delete; the
//                                test entries are removed at the end.  Without a store (WSL has
//                                no keyring): Unavailable everywhere, and no file instead.
//   plattest --test secret-bus --mode M
//                                (Linux; tools/platshared/secret_bus.py starts it against its
//                                fake Secret Service, which checks the other side) dh: the
//                                round trip of --test secret; plain: the service lacks the
//                                encrypted algorithm; locked: the collection locks itself after
//                                every call, each call unlocks it through a prompt; dismiss: the
//                                user dismisses the prompts; nodefault: no default collection
//                                (CreateCollection and its prompt), prompts for CreateItem and
//                                Delete; badkey: an invalid public key; none: no Secret Service
//                                on the bus
// data:// is mounted on the scratch folder first: the DPAPI files land there, and the checks
// can see that nothing else was written.

use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;

use plat::secret::{SECRET_MAX, SECRET_NAME_MAX, SecretError, secret_available, secret_delete, secret_get, secret_get_checked, secret_set};

use crate::T;

// data:// on <scratch>/data -> its OS path
fn scratch_data(t: &mut T) -> String {
    t.scratch_init();
    let dir = format!("{}/{}/data", plat::cwd().replace('\\', "/"), t.base);
    let ok = plat::mkdir(&dir) && plat::vfs::mount("data", Arc::new(plat::vfs::DirFs::new(&dir)));
    t.check(ok, "secret: data:// on the scratch folder");
    dir
}

// the files below data://secrets
fn dpapi_files() -> Vec<String> {
    let mut v = Vec::new();
    plat::dir_list("data://secrets", &mut |name, _, _| v.push(String::from(name)));
    v
}

// every entry below dir (recursively), for "nothing was written"
fn count_tree(dir: &str) -> u32 {
    let mut subs = Vec::new();
    let mut n = 0;
    plat::dir_list(dir, &mut |name, is_dir, _| {
        n += 1;
        if is_dir {
            subs.push(format!("{dir}/{name}"));
        }
    });
    n + subs.iter().map(|s| count_tree(s)).sum::<u32>()
}

fn is(v: Option<Vec<u8>>, want: &[u8]) -> bool {
    v.as_deref() == Some(want)
}

pub fn t_secret(t: &mut T) {
    let dir = scratch_data(t);
    let svc = format!("plattest-{:08x}", plat::random_u64() as u32);
    t.info(&format!("  service {svc}\n"));
    if !secret_available() {
        t.check(cfg!(target_os = "linux"), "secret: no store only on Linux (Windows always has the Credential Manager)");
        unavailable(t, &svc, &dir);
    } else {
        round_trip(t, &svc, true);
    }
    t.scratch_done();
}

/// no store: every call fails cleanly, and nothing is written anywhere instead
fn unavailable(t: &mut T, svc: &str, dir: &str) {
    t.info("  no secret store on this system\n");
    t.check(secret_set(svc, "alice", b"hunter2") == Err(SecretError::Unavailable), "secret: set without a store -> Unavailable");
    t.check(secret_get(svc, "alice").is_none(), "secret: get without a store -> None");
    t.check(secret_get_checked(svc, "alice") == Err(SecretError::Unavailable), "secret: get_checked without a store -> Unavailable");
    t.check(secret_delete(svc, "alice") == Err(SecretError::Unavailable), "secret: delete without a store -> Unavailable");
    t.expect_i(count_tree(dir) as i64, 0, "secret: no file written instead (data:// stays empty)");
}

/// the whole contract against a working store; large: values above the Credential Manager's
/// limit (DPAPI files on Windows)
fn round_trip(t: &mut T, svc: &str, large: bool) {
    let app = plat::vfs::app_id();
    let first: &[u8] = b"hunter2";
    let second: &[u8] = b"correct horse battery staple";
    t.check(secret_get(svc, "alice").is_none() && secret_get_checked(svc, "alice") == Ok(None), "secret: nothing there yet (Ok(None))");
    t.check(secret_set(svc, "alice", first).is_ok(), "secret: set");
    t.check(is(secret_get(svc, "alice"), first), "secret: get returns the value");
    t.check(secret_set(svc, "alice", second).is_ok(), "secret: overwrite");
    t.check(is(secret_get(svc, "alice"), second), "secret: get returns the new value");
    let bin: Vec<u8> = (0..=255u8).rev().collect();
    t.check(secret_set(svc, "bob", &bin).is_ok() && is(secret_get(svc, "bob"), &bin), "secret: a binary value (all 256 bytes) for a second account");
    t.check(is(secret_get(svc, "alice"), second), "secret: the first account is unchanged");
    t.check(secret_set(svc, "", b"api-key").is_ok() && is(secret_get(svc, ""), b"api-key"), "secret: an empty account");
    t.check(secret_set(svc, "empty", b"").is_ok() && is(secret_get(svc, "empty"), b""), "secret: an empty value");
    let slashed = format!("{svc}/x");
    t.check(secret_set(&slashed, "y", b"one").is_ok() && secret_set(svc, "x/y", b"two").is_ok(), "secret: set 'svc/x' + 'y' and 'svc' + 'x/y'");
    t.check(is(secret_get(&slashed, "y"), b"one") && is(secret_get(svc, "x/y"), b"two"), "secret: ... they are two entries");
    t.check(secret_set(svc, "ü ✓ 名前", "wert ✓".as_bytes()).is_ok() && is(secret_get(svc, "ü ✓ 名前"), "wert ✓".as_bytes()), "secret: Unicode names");

    // another app does not see plattest's entries; its own entry of the same name is apart
    plat::vfs::set_app_id("plattest-other");
    t.check(plat::vfs::app_id() == "plattest-other", "secret: app id switched to plattest-other");
    t.check(secret_get(svc, "alice").is_none(), "secret: another app does not read the entry");
    t.check(secret_delete(svc, "alice") == Ok(false), "secret: ... nor delete it");
    t.check(secret_set(svc, "alice", b"other app").is_ok() && is(secret_get(svc, "alice"), b"other app"), "secret: the other app's own entry of that name");
    t.check(secret_delete(svc, "alice") == Ok(true), "secret: the other app deletes its entry");
    plat::vfs::set_app_id(&app);
    t.check(is(secret_get(svc, "alice"), second), "secret: plattest's entry is unchanged");

    if large {
        let big = |seed: u32, n: usize| -> Vec<u8> { (0..n as u32).map(|i| (i.wrapping_mul(2654435761).wrapping_add(seed) >> 13) as u8).collect() };
        let win = cfg!(windows);
        let (a, b) = (big(1, 40_000), big(2, 3_000));
        t.check(secret_set(svc, "big", &a).is_ok() && is(secret_get(svc, "big"), &a), "secret: a 40000-byte value");
        if win {
            let f = dpapi_files();
            t.check(f.len() == 1, "secret: ... is one DPAPI file below data://secrets");
            if let Some(name) = f.first() {
                let p = format!("data://secrets/{name}");
                let raw = plat::read_file(&p).unwrap_or_default();
                t.check(!raw.is_empty() && !raw.windows(64).any(|w| w == &a[..64]), "secret: ... which does not hold the value in plain");
                let mut bad = raw.clone();
                if let Some(x) = bad.last_mut() {
                    *x ^= 1;
                }
                plat::write_file(&p, &bad);
                t.check(matches!(secret_get_checked(svc, "big"), Err(SecretError::Failed(_))), "secret: a changed DPAPI file is refused");
                plat::write_file(&p, &raw);
                t.check(is(secret_get(svc, "big"), &a), "secret: the restored file is accepted");
            }
        }
        t.check(secret_set(svc, "big", &b).is_ok() && is(secret_get(svc, "big"), &b), "secret: overwrite with 3000 bytes");
        if win {
            t.expect_i(dpapi_files().len() as i64, 1, "secret: ... the old DPAPI file is gone, the new one is there");
        }
        t.check(secret_set(svc, "big", b"small").is_ok() && is(secret_get(svc, "big"), b"small"), "secret: overwrite with a small value");
        if win {
            t.expect_i(dpapi_files().len() as i64, 0, "secret: ... no DPAPI file left");
        }
        let edge = big(3, 2560);
        t.check(secret_set(svc, "edge", &edge).is_ok() && is(secret_get(svc, "edge"), &edge), "secret: exactly 2560 bytes");
        let max = big(4, SECRET_MAX);
        t.check(secret_set(svc, "big", &max).is_ok() && is(secret_get(svc, "big"), &max), "secret: SECRET_MAX bytes");
        t.check(secret_set(svc, "big", &big(5, SECRET_MAX + 1)) == Err(SecretError::Invalid), "secret: SECRET_MAX + 1 bytes -> Invalid");
        t.check(is(secret_get(svc, "big"), &max), "secret: ... the stored value is unchanged");
        t.check(secret_delete(svc, "big") == Ok(true) && secret_delete(svc, "edge") == Ok(true), "secret: delete the large entries");
        if win {
            t.expect_i(dpapi_files().len() as i64, 0, "secret: ... and their DPAPI files");
        }
    }

    // invalid names
    let long = "x".repeat(SECRET_NAME_MAX + 1);
    t.check(secret_set("", "a", b"x") == Err(SecretError::Invalid), "secret: an empty service -> Invalid");
    t.check(secret_set(svc, "a\0b", b"x") == Err(SecretError::Invalid) && secret_get_checked(&long, "a") == Err(SecretError::Invalid),
        "secret: NUL and too long names -> Invalid");
    t.check(secret_set(&"s".repeat(SECRET_NAME_MAX), &"a".repeat(SECRET_NAME_MAX), b"x").is_ok(), "secret: names of SECRET_NAME_MAX bytes");
    t.check(secret_delete(&"s".repeat(SECRET_NAME_MAX), &"a".repeat(SECRET_NAME_MAX)) == Ok(true), "secret: ... deleted");

    // delete, then clean up
    t.check(secret_delete(svc, "alice") == Ok(true), "secret: delete");
    t.check(secret_get(svc, "alice").is_none() && secret_get_checked(svc, "alice") == Ok(None), "secret: gone after delete");
    t.check(secret_delete(svc, "alice") == Ok(false), "secret: delete of a missing entry -> Ok(false)");
    let rest = [(svc, "bob"), (svc, ""), (svc, "empty"), (slashed.as_str(), "y"), (svc, "x/y"), (svc, "ü ✓ 名前")];
    let mut left = 0;
    for (s, a) in rest {
        let _ = secret_delete(s, a);
        if secret_get(s, a).is_some() {
            left += 1;
        }
    }
    for (s, a) in [(svc, "big"), (svc, "edge"), (svc, "alice")] {
        let _ = secret_delete(s, a);
    }
    t.expect_i(left, 0, "secret: every test entry removed");
}

#[cfg(windows)]
pub fn t_secret_bus(_: &mut T) {
    plat::write_stderr(b"plattest: --test secret-bus is Linux only\n");
    plat::exit(2);
}

#[cfg(target_os = "linux")]
pub fn t_secret_bus(t: &mut T) {
    let args = base::Args(plat::args());
    let mode = String::from(args.value("--mode").unwrap_or("dh"));
    let dir = scratch_data(t);
    let svc = "secret-bus";
    t.info(&format!("  mode {mode}\n"));
    match mode.as_str() {
        "dh" | "plain" => {
            t.check(secret_available(), "secret-bus: secret_available");
            round_trip(t, svc, true);
        }
        "locked" => {
            // the fake holds (plattest, secret-bus, locked) = "from the keyring", locked
            t.check(is(secret_get(svc, "locked"), b"from the keyring"), "secret-bus: get unlocks the item through a prompt");
            t.check(secret_set(svc, "new", b"stored while locked").is_ok(), "secret-bus: set unlocks the collection (IsLocked, Unlock, prompt)");
            t.check(is(secret_get(svc, "new"), b"stored while locked"), "secret-bus: ... and the value is there");
            t.check(secret_delete(svc, "new") == Ok(true) && secret_delete(svc, "locked") == Ok(true), "secret-bus: delete unlocks the items first");
            t.check(secret_get_checked(svc, "locked") == Ok(None), "secret-bus: both gone");
        }
        "dismiss" => {
            t.check(secret_get(svc, "locked").is_none(), "secret-bus: a dismissed unlock -> get None");
            t.check(secret_get_checked(svc, "locked") == Err(SecretError::Dismissed), "secret-bus: ... get_checked Dismissed");
            t.check(secret_set(svc, "new", b"x") == Err(SecretError::Dismissed), "secret-bus: set into a locked collection, dismissed");
            t.check(secret_delete(svc, "locked") == Err(SecretError::Dismissed), "secret-bus: delete, dismissed");
        }
        "nodefault" => {
            t.check(secret_set(svc, "alice", b"first entry").is_ok(), "secret-bus: set creates the default collection (prompt), then the item (prompt)");
            t.check(is(secret_get(svc, "alice"), b"first entry"), "secret-bus: get");
            t.check(secret_set(svc, "alice", b"second").is_ok() && is(secret_get(svc, "alice"), b"second"), "secret-bus: overwrite in the new collection");
            t.check(secret_delete(svc, "alice") == Ok(true) && secret_get(svc, "alice").is_none(), "secret-bus: delete with a prompt");
        }
        "badkey" => {
            t.check(matches!(secret_set(svc, "alice", b"x"), Err(SecretError::Failed(_))), "secret-bus: an invalid public key of the service -> Failed");
            t.check(matches!(secret_get_checked(svc, "alice"), Err(SecretError::Failed(_))), "secret-bus: ... also for get");
        }
        "none" => {
            t.check(!secret_available(), "secret-bus: no Secret Service on the bus -> not available");
            unavailable(t, svc, &dir);
        }
        _ => t.check(false, &format!("secret-bus: unknown mode {mode}")),
    }
    if mode != "none" {
        t.expect_i(count_tree(&dir) as i64, 0, "secret-bus: nothing written to data://");
    }
    t.scratch_done();
    crate::out("secret-bus: done\n");
}
