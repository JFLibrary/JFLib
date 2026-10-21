// t_instance -- plattest --test instance: one running instance per app and user
// (lib/plat/instance.rs)
//
//   Request::paths, claim of a fresh app name (Primary), send from this process, a second
//   process (spawn_self --test instance-client) that is Forwarded and whose arguments arrive
//   here, and a second claim in this process (Forwarded to itself).
//   plattest --test instance-client --inst NAME ...   that second process: exit code 0 when
//   claim(NAME) was Forwarded

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use plat::instance::{self, Claim, Request};

use crate::T;

fn s(v: &[&str]) -> Vec<String> {
    v.iter().map(|x| String::from(*x)).collect()
}

// the next request, waited for up to 5 s
fn next() -> Option<Request> {
    for _ in 0..500 {
        let mut v = instance::take();
        if !v.is_empty() {
            return Some(v.remove(0));
        }
        plat::sleep_ms(10);
    }
    None
}

pub fn t_instance(t: &mut T) {
    let r = Request { cwd: String::from("/home/u/"), args: s(&["-v", "a.txt", "/abs", "C:/x", "config://s.ini", "https://e.org/x", "--k=v"]) };
    t.check(r.paths() == s(&["/home/u/a.txt", "/abs", "C:/x", "config://s.ini", "https://e.org/x"]), "instance: Request::paths");

    let name = format!("plattest-inst-{:x}", plat::time_us() & 0xFFFF_FFFF);
    t.check(instance::claim(&name) == Claim::Primary, "instance: the first claim is the instance");
    t.check(instance::claim("bad name/x") == Claim::Alone, "instance: an invalid app name stays alone");

    t.check(instance::send(&name, "C:/w", &s(&["a.txt", "--x", "ü"])), "instance: send from this process");
    let got = next();
    t.check(got == Some(Request { cwd: String::from("C:/w"), args: s(&["a.txt", "--x", "ü"]) }), "instance: the request arrives");

    // a second process with the same app name hands its command line over and ends
    let started = plat::spawn_self(&["--test", "instance-client", "--inst", &name, "rel/file.txt"]);
    t.check(started == 0, "instance: a second process starts");
    let got = next();
    let cwd = plat::cwd();
    t.check(got.as_ref().is_some_and(|r| r.args.iter().any(|a| a == "rel/file.txt") && r.cwd == cwd), "instance: its arguments and folder arrive");
    let want = format!("{}/rel/file.txt", cwd.trim_end_matches(['/', '\\']));
    t.check(got.as_ref().is_some_and(|r| r.paths().contains(&want)), "instance: paths() resolves against its folder");

    // a second claim in this process reaches the first one
    t.check(instance::claim(&name) == Claim::Forwarded, "instance: a later claim is forwarded");
    t.check(next().is_some_and(|r| r.args == plat::args().into_iter().skip(1).collect::<Vec<_>>()), "instance: with this process's arguments");

    t.check(!instance::send(&format!("{name}-none"), "/", &s(&["x"])), "instance: no instance, no delivery");
}

/// the second process of t_instance: exit code 0 when claim was Forwarded
pub fn t_instance_client(t: &mut T) {
    let name = plat::arg_value("--inst").unwrap_or_default();
    let c = instance::claim(&name);
    t.check(c == Claim::Forwarded, "instance-client: forwarded to the running instance");
}
