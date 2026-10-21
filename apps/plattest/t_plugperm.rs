// t_plugperm -- plattest --test plugperm: the rights of native plugins (lib/plat/policy.rs)
//
//   1. a plugin's manifest "<stem>.permissions" beside the library: unrestricted without one
//      while the app has no manifest; no ask / agent lines; $PLUGIN = its folder
//   2. with the app's manifest installed: calls on behalf of a plugin (policy::on_behalf) need
//      the plugin's rules AND the app's -- files, the network, processes, URLs; the refusal
//      names the plugin; a plugin without a manifest gets nothing; nested scopes intersect;
//      jobs on the pool (spawn_blocking, parallel_for) keep the scope; outside it the app's
//      rules alone
// A policy stays installed until the process ends: this test is not part of "all".

use alloc::format;
use core::sync::atomic::{AtomicU32, Ordering::SeqCst};

use plat::net::{IpAddr, SocketAddr};
use plat::policy;

use crate::{T, rm_tree};

pub fn t_plugperm(t: &mut T) {
    let cwd = plat::cwd();
    let base = format!("{}/build/plattest/{}-plugperm-{:x}", cwd.replace('\\', "/"), crate::OS, plat::time_us() & 0xFF_FFFF);
    let p = |s: &str| format!("{base}/{s}");
    let ok = plat::mkdir(&p("plugins/data")) && plat::mkdir(&p("plugins/out")) && plat::mkdir(&p("app"))
        && plat::write_file(&p("plugins/data/in.txt"), b"in") && plat::write_file(&p("app/doc.txt"), b"doc")
        && plat::write_file(&p("plugins/good.permissions"),
            format!("# what the plugin needs\nfs.read $PLUGIN/data/**\nfs.write $PLUGIN/out/**\nfs.read {base}/app/**\n\
                     fs.write {base}/elsewhere/**\nnet.connect 127.0.0.1:7\nprocess plattest-helper\n").as_bytes())
        && plat::write_file(&p("plugins/other.permissions"), b"fs.write $PLUGIN/data/**\n")
        && plat::write_file(&p("plugins/bad.permissions"), b"fs.read /x\nask fs.read /y\n")
        && plat::write_file(&p("plugins/agent.permissions"), b"agent.deny doc.x\n");
    t.check(ok, "plugperm: scratch tree");

    // no app manifest yet: a plugin without one is unrestricted
    let free = policy::plugin_rights(&p("plugins/free.dll"));
    t.check(free.as_ref().is_ok_and(|r| !r.restricted() && r.name() == "free"), "plugperm: no manifests -> unrestricted");
    let bad = policy::plugin_rights(&p("plugins/bad.dll"));
    t.check(bad.as_ref().err().is_some_and(|e| e.contains("bad.permissions:2: a plugin cannot ask")), "plugperm: a plugin cannot ask");
    let agent = policy::plugin_rights(&p("plugins/agent.so"));
    t.check(agent.as_ref().err().is_some_and(|e| e.contains("agent rules belong")), "plugperm: no agent rules in a plugin's manifest");

    let manifest = format!(
        "# plattest --test plugperm\nfs.write {base}/plugins/**\nfs.read {base}/app/**\n\
         net.connect 127.0.0.1:7-8\nprocess plattest-helper plattest-other\nurl https\n"
    );
    let key = match policy::install(&manifest) {
        Ok(k) => k,
        Err(e) => {
            t.check(false, &format!("plugperm: install ({e})"));
            return;
        }
    };
    let Ok(good) = policy::plugin_rights(&p("plugins/good.dll")) else {
        t.check(false, "plugperm: good.permissions parses");
        return;
    };
    t.check(good.restricted() && good.name() == "good", "plugperm: good.permissions parses");
    let none = policy::plugin_rights(&p("plugins/none.dll"));
    t.check(none.as_ref().is_ok_and(|r| r.restricted()), "plugperm: no manifest beside it -> no rights (the app has one)");
    let Ok(other) = policy::plugin_rights(&p("plugins/other.so")) else {
        t.check(false, "plugperm: other.permissions parses");
        return;
    };

    let addr = |port| SocketAddr::new(IpAddr::LOCALHOST_V4, port);
    policy::on_behalf(&good, || {
        t.check(policy::plugin_scope().is_some_and(|s| s.name() == "good"), "plugperm: plugin_scope inside on_behalf");
        t.check(plat::read_file(&p("plugins/data/in.txt")).is_some(), "plugperm: read $PLUGIN/data (both allow)");
        t.check(plat::write_file(&p("plugins/out/o.txt"), b"o"), "plugperm: write $PLUGIN/out (both allow)");
        t.check(!plat::write_file(&p("plugins/data/x.txt"), b"x"), "plugperm: the app allows, the plugin only reads -> refused");
        t.check(plat::read_file(&p("app/doc.txt")).is_some(), "plugperm: the app's folder (both allow)");
        t.check(!plat::mkdir(&p("elsewhere")), "plugperm: the plugin allows, the app does not -> refused");
        t.check(policy::net_connect_allowed(&addr(7)), "plugperm: net.connect in both");
        t.check(!policy::net_connect_allowed(&addr(8)), "plugperm: net.connect only in the app's rules");
        t.check(policy::process_allowed("plattest-helper") && !policy::process_allowed("plattest-other"), "plugperm: processes");
        t.check(!policy::url_allowed("https://example.org"), "plugperm: no url rule in the plugin");
    });
    t.check(policy::plugin_scope().is_none(), "plugperm: the scope ends with on_behalf");
    let d = policy::denials();
    t.check(d.iter().any(|x| x.kind == "fs.write" && x.target == p("plugins/data/x.txt") && x.plugin == "good"), "plugperm: the refusal names the plugin");
    t.check(plat::write_file(&p("plugins/data/x.txt"), b"x") && policy::url_allowed("https://example.org"), "plugperm: outside, the app's rules alone");

    if let Ok(none) = &none {
        policy::on_behalf(none, || {
            t.check(plat::read_file(&p("plugins/data/in.txt")).is_none() && plat::read_file(&p("app/doc.txt")).is_none(),
                "plugperm: a plugin without a manifest reads nothing");
            t.check(!policy::net_connect_allowed(&addr(7)), "plugperm: ... and connects nowhere");
        });
    }

    // nested scopes: all of them
    policy::on_behalf(&good, || {
        policy::on_behalf(&other, || {
            t.check(policy::plugin_scope().is_some_and(|s| s.name() == "other"), "plugperm: the innermost scope");
            t.check(!plat::write_file(&p("plugins/data/y.txt"), b"y"), "plugperm: nested: the outer plugin refuses");
            t.check(plat::read_file(&p("plugins/out/o.txt")).is_some(), "plugperm: nested: both read (the folder of the inner one is implied)");
            t.check(plat::read_file(&p("app/doc.txt")).is_none(), "plugperm: nested: the inner plugin refuses");
        });
        t.check(plat::read_file(&p("app/doc.txt")).is_some(), "plugperm: back in the outer scope");
    });

    // jobs on the pool keep the scope
    let job = policy::on_behalf(&good, || {
        let w = p("plugins/data/z.txt");
        plat::task::spawn_blocking(move || plat::write_file(&w, b"z"))
    });
    t.check(!plat::task::block_on(job), "plugperm: spawn_blocking keeps the plugin's scope");
    let (wrote, read) = (AtomicU32::new(0), AtomicU32::new(0));
    policy::on_behalf(&good, || {
        let _ = plat::parallel_for(16, &|i, _| {
            if plat::write_file(&p(&format!("plugins/data/p{i}.txt")), b"p") {
                wrote.fetch_add(1, SeqCst);
            }
            if plat::read_file(&p("plugins/data/in.txt")).is_some() {
                read.fetch_add(1, SeqCst);
            }
        });
    });
    t.check(wrote.load(SeqCst) == 0 && read.load(SeqCst) == 16, "plugperm: parallel_for items keep the scope on every worker");
    key.trusted(|| rm_tree(&base));
}
