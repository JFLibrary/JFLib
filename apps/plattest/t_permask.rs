// t_permask -- plattest --test permask: questions at run time without the UI (lib/plat/policy.rs)
//
//   1. the syntax of ask lines, reasons and agent lines (policy::check)
//   2. a manifest with ask lines installed in this process: nobody asks (refused, no question),
//      a plain rule wins over an ask line, fixed answers (Key::set_answer), the asking thread
//      (Pending, Key::answer, the decision for the line), another thread waiting for the answer
//      and refused after the timeout, Key::ask_here(false) withdrawing a question, the stored
//      answers (a file: written, protected from the app, reloaded, forgotten), decisions(),
//      agent rules and questions, the network's name check
// A policy stays installed until the process ends: this test is not part of "all".

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use core::sync::atomic::{AtomicU32, Ordering::SeqCst};

use plat::policy::{self, AgentRule, Answer, Verdict};

use crate::{T, rm_tree};

pub fn t_permask(t: &mut T) {
    syntax(t);
    installed(t);
}

fn syntax(t: &mut T) {
    let bad: [(&str, &str); 11] = [
        ("ask fs.user", "PERMISSIONS:1: fs.user cannot be asked for"),
        ("print\nask", "PERMISSIONS:2: ask needs a permission"),
        ("fs.read /x \"why\"", "a reason belongs to an ask line"),
        ("ask url mailto \"open", "no closing quote"),
        ("ask url mailto \"a\" b", "nothing but a comment may follow"),
        ("ask agent.deny doc.x", "agent.deny cannot be asked for"),
        ("agent.confirm doc/x", "is not an action name pattern"),
        ("agent.deny", "expected at least one action name"),
        ("fs.read $PLUGIN/x", "$PLUGIN only in a plugin's manifest"),
        ("ask print \"a\u{7}b\"", "no control characters"),
        ("sandbox loose", "sandbox takes nothing or 'strict'"),
    ];
    for (text, want) in bad {
        match policy::check(text) {
            Err(e) => {
                let ok = e.contains(want);
                t.check(ok, &format!("permask: refused {:?}", text));
                if !ok {
                    t.info(&format!("  got {e}\n"));
                }
            }
            Ok(()) => t.check(false, &format!("permask: refused {:?}", text)),
        }
    }
    let good = "ask fs.read $TEMP/x/** \"Briefe # öffnen\"   # a comment\nask net.connect *:443\nask print\n\
                ask url mailto https\nagent.confirm doc.* ui.type\nagent.deny *.wipe\n\"\"";
    let r = policy::check(good);
    t.check(r.is_err() && r.unwrap_err().contains("PERMISSIONS:7: a reason belongs"), "permask: a quote alone is a reason without a line");
    t.check(policy::check(&good[..good.len() - 3]).is_ok(), "permask: ask lines, reasons with '#', agent lines");
    t.check(policy::check("sandbox\nsandbox strict\n").is_ok(), "permask: the sandbox line (with 'strict')");
}

fn installed(t: &mut T) {
    let cwd = plat::cwd();
    let base = format!("{}/build/plattest/{}-permask-{:x}", cwd.replace('\\', "/"), crate::OS, plat::time_us() & 0xFF_FFFF);
    let p = |s: &str| format!("{base}/{s}");
    let ok = ["plain", "ask", "bg", "both"].iter().all(|d| plat::mkdir(&p(d)))
        && plat::write_file(&p("ask/a.txt"), b"a") && plat::write_file(&p("both/x.txt"), b"x");
    t.check(ok, "permask: scratch tree");
    let manifest = format!(
        "# plattest --test permask\nfs.write {base}/plain/**\n\
         ask fs.read {base}/ask/**  \"Briefe lesen\"\n\
         ask fs.write {base}/bg/**\n\
         ask fs.read {base}/both/**\nfs.read {base}/both/**   # the plain line wins\n\
         ask url mailto \"Mails schreiben\"\nask print\n\
         ask net.connect *.ask.test:443\n\
         agent.confirm doc.* ui.type\nagent.deny *.wipe\n"
    );
    let key = match policy::install(&manifest) {
        Ok(k) => k,
        Err(e) => {
            t.check(false, &format!("permask: install ({e})"));
            return;
        }
    };
    key.set_store(None); // memory until the file part below
    let rule_ask = format!("fs.read {base}/ask/**");
    let rule_bg = format!("fs.write {base}/bg/**");
    let answer_of = |rule: &str| policy::decisions().into_iter().find(|d| d.rule == rule).and_then(|d| d.answer);

    // nobody asks: refused, no question, no decision
    t.check(plat::read_file(&p("ask/a.txt")).is_none(), "permask: nobody asks -> refused");
    t.check(policy::questions().is_empty(), "permask: ... without a question");
    t.check(answer_of(&rule_ask).is_none(), "permask: ... and without a decision");
    t.check(policy::denials().iter().any(|d| d.kind == "fs.read" && d.target == p("ask/a.txt") && d.plugin.is_empty()), "permask: ... recorded");
    t.check(plat::read_file(&p("both/x.txt")).is_some(), "permask: a plain line wins over an ask line");
    t.check(plat::write_file(&p("plain/y.txt"), b"y"), "permask: plain lines are not asked");
    let d = policy::decisions();
    t.check(d.len() == 6 && d.iter().all(|x| x.answer.is_none()), "permask: decisions() lists every ask line");
    t.check(d.iter().any(|x| x.rule == "url mailto" && x.kind == "url" && x.reason == "Mails schreiben"), "permask: ... with its kind and reason");

    // fixed answers (jfgui_PERM_ANSWER, the scripts' perm answer)
    key.set_answer(Some(Answer::Once));
    t.check(plat::read_file(&p("ask/a.txt")).is_some(), "permask: answer once -> allowed");
    t.check(answer_of(&rule_ask) == Some(Answer::Once), "permask: ... decided for the line");
    t.check(plat::file_exists(&p("ask/none.txt")) == 0 && answer_of(&rule_ask) == Some(Answer::Once), "permask: ... for the whole line");
    key.set_answer(Some(Answer::Deny));
    t.check(!policy::url_allowed("mailto:a@b.example"), "permask: answer deny -> refused");
    key.set_answer(None);
    t.check(!policy::url_allowed("mailto:c@d.example"), "permask: deny holds until the end");
    t.check(policy::forget("url mailto") && answer_of("url mailto").is_none(), "permask: forget");
    t.check(!policy::forget("url mailto"), "permask: forget without a decision");

    // the asking thread: refused while the question is open, then the answer decides
    key.ask_here(true);
    t.check(!policy::url_allowed("mailto:e@f.example"), "permask: the asking thread is refused while it asks");
    let qs = policy::questions();
    t.check(qs.len() == 1, "permask: one question");
    if let Some(q) = qs.first() {
        t.check(q.kind == "url" && q.target == "mailto:e@f.example" && q.rule == "url mailto" && q.reason == "Mails schreiben" && q.plugin.is_empty(),
            "permask: the question: kind, target, line, reason");
        t.check(!policy::url_allowed("mailto:g@h.example") && policy::questions().len() == 1, "permask: one question per line");
        t.check(key.answer(q.id, Answer::Once), "permask: answer");
        t.check(!key.answer(q.id, Answer::Once), "permask: a question is answered once");
    }
    t.check(policy::url_allowed("mailto:e@f.example") && policy::questions().is_empty(), "permask: allowed after the answer");

    // another thread waits for the answer
    let result = Arc::new(AtomicU32::new(0));
    let r2 = result.clone();
    let path = p("bg/b.txt");
    let th = plat::thread_spawn(0, Box::new(move || {
        r2.store(if plat::write_file(&path, b"bg") { 1 } else { 2 }, SeqCst);
    }));
    let asked = wait_for(|| !policy::questions().is_empty(), 5000);
    t.check(asked, "permask: another thread's question reaches the asking one");
    t.check(result.load(SeqCst) == 0, "permask: ... and that thread waits");
    if let Some(q) = policy::questions().first() {
        t.check(q.rule == rule_bg && q.kind == "fs.write", "permask: ... for its line");
        key.answer(q.id, Answer::Always);
    }
    if let Some(h) = th {
        h.join();
    }
    t.check(result.load(SeqCst) == 1, "permask: the waiting thread continues with the answer");
    t.check(answer_of(&rule_bg) == Some(Answer::Always), "permask: always decided");

    // ... and is refused after the timeout; the question stays until it is withdrawn
    policy::forget(&rule_bg);
    key.set_wait(200);
    let result = Arc::new(AtomicU32::new(0));
    let r3 = result.clone();
    let path = p("bg/c.txt");
    let t0 = plat::time_us();
    let th = plat::thread_spawn(0, Box::new(move || {
        r3.store(if plat::write_file(&path, b"bg") { 1 } else { 2 }, SeqCst);
    }));
    if let Some(h) = th {
        h.join();
    }
    let ms = (plat::time_us() - t0) / 1000;
    t.check(result.load(SeqCst) == 2 && (150..5000).contains(&ms), "permask: refused after the timeout");
    t.info(&format!("  waited {ms} ms\n"));
    t.check(policy::questions().len() == 1 && answer_of(&rule_bg).is_none(), "permask: the question stays open, nothing decided");
    // withdrawn: nobody asks any more
    let result = Arc::new(AtomicU32::new(0));
    let r4 = result.clone();
    key.set_wait(10_000);
    let path = p("bg/d.txt");
    let th = plat::thread_spawn(0, Box::new(move || {
        r4.store(if plat::write_file(&path, b"bg") { 1 } else { 2 }, SeqCst);
    }));
    plat::sleep_ms(100);
    key.ask_here(false);
    if let Some(h) = th {
        h.join();
    }
    t.check(result.load(SeqCst) == 2 && policy::questions().is_empty(), "permask: ask_here(false) withdraws the question, the waiter is refused");

    // the network: names are asked for when they are looked up
    key.set_answer(Some(Answer::Deny));
    t.check(!policy::net_name_allowed("www.ask.test") && !policy::net_name_allowed("other.test"), "permask: net.connect asked at the name lookup");
    t.check(answer_of("net.connect *.ask.test:443") == Some(Answer::Deny), "permask: ... decided for the line");

    // agent rules
    t.check(policy::agent_rule("doc.save") == AgentRule::Confirm && policy::agent_rule("ui.type") == AgentRule::Confirm, "permask: agent.confirm patterns");
    t.check(policy::agent_rule("all.wipe") == AgentRule::Deny && policy::agent_rule("ui.click") == AgentRule::Free, "permask: agent.deny, free actions");
    key.set_answer(None);
    t.check(policy::agent_ask("doc.save", "Save") == Verdict::Deny, "permask: an agent question with nobody to ask");
    key.ask_here(true);
    t.check(policy::agent_ask("doc.save", "Save the document") == Verdict::Pending, "permask: ... pending on the asking thread");
    if let Some(q) = policy::questions().first() {
        t.check(q.kind == "agent" && q.target == "doc.save" && q.rule == "agent doc.save" && q.reason == "Save the document", "permask: the agent's question");
        key.answer(q.id, Answer::Never);
    }
    t.check(policy::agent_ask("doc.save", "") == Verdict::Deny, "permask: never for the agent");
    t.check(policy::decisions().iter().any(|d| d.rule == "agent doc.save" && d.kind == "agent" && d.answer == Some(Answer::Never) && d.reason == "Save the document"),
        "permask: agent decisions listed with the description");
    key.ask_here(false);

    // the stored answers: a file (where the app may write otherwise), protected, reloaded, forgotten
    let store = p("plain/answers.txt");
    key.set_store(Some(&store));
    policy::forget_all();
    key.set_answer(Some(Answer::Always));
    t.check(policy::print_allowed(), "permask: always");
    key.set_answer(Some(Answer::Never));
    t.check(!policy::url_allowed("mailto:x@y.example"), "permask: never");
    key.set_answer(None);
    let text = String::from_utf8(key.trusted(|| plat::read_file(&store)).unwrap_or_default()).unwrap_or_default();
    t.check(text.contains("always print\n") && text.contains("never url mailto\n"), "permask: written to the file");
    t.check(!text.contains(&rule_ask), "permask: once / deny are not stored");
    t.check(!plat::write_file(&store, b"always fs.read /\n"), "permask: the app cannot write the file");
    policy::reload();
    t.check(answer_of("print") == Some(Answer::Always) && answer_of("url mailto") == Some(Answer::Never), "permask: reloaded");
    policy::forget("print");
    policy::reload();
    t.check(answer_of("print").is_none() && answer_of("url mailto") == Some(Answer::Never), "permask: forget removes it from the file");
    policy::forget_all();
    t.check(policy::decisions().iter().all(|d| d.answer.is_none()), "permask: forget_all");
    key.set_store(None);
    key.trusted(|| rm_tree(&base));
}

// poll cond for up to ms
fn wait_for(cond: impl Fn() -> bool, ms: u64) -> bool {
    let end = plat::time_us() + ms * 1000;
    while !cond() {
        if plat::time_us() >= end {
            return false;
        }
        plat::sleep_ms(5);
    }
    true
}
