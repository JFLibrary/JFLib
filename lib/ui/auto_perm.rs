// auto_perm -- the script commands for permission questions and agent control (auto.rs)
//
//   perm answer once|always|deny|never   every question answered so, without a dialog
//   perm answer dialog            the questions show as the library's dialog (click its
//                                 buttons); a script starts without one: nobody asks, refused
//   perm answer none              nobody asks again (the headless default)
//   perm timeout MS               how long other threads wait for an answer
//   perm store FILE|none          where Always / Never answers are kept (scripts start with
//                                 none: memory only, unless jfgui_PERM_STORE is set)
//   perm reload                   read the stored answers again (a later run of the app)
//   perm wait [MS]                wait (real time, default 5 s) until a question is open, then
//                                 run the passes that show it
//   perm reset                    forget every answer (this run's and the stored ones)
//   perm expect open | none       a question is open / none is
//   perm expect RULE once|always|deny|never|unanswered   the answer of an ask line
//                                 ("fs.read $TEMP/x/**") or an agent action ("agent doc.save")
//   agent on | off                external control on / off (no endpoint is opened)
//   agent call ACTION [JSON]      actions/execute as an external client (JSON-RPC, protocol 1;
//                                 the input object, default {})
//   agent expect ok | error CODE  the outcome of the last call (forbidden,
//                                 confirmation_required, not_found, ...)

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use plat::policy::{self, Answer};

use super::{App, Ui};

/// the agent session of a script
#[derive(Default)]
pub(crate) struct S {
    session: Option<agent_core::RpcSession>,
    reply: Option<String>,
    calls: u64,
}

impl Ui {
    pub(crate) fn sc_perm(&mut self, app: &mut dyn App, a: &[u8]) {
        let line = String::from_utf8_lossy(a).into_owned();
        let w: Vec<&str> = line.split_whitespace().collect();
        match w.first().copied() {
            Some("answer") => match w.get(1).copied() {
                Some("dialog") => {
                    super::perm::set_answer(None);
                    super::perm::ask_here(true);
                }
                Some("none") => {
                    super::perm::set_answer(None);
                    super::perm::ask_here(false);
                }
                Some(x) => match Answer::parse(x) {
                    Some(ans) => super::perm::set_answer(Some(ans)),
                    None => self.fail(b"perm answer once|always|deny|never|dialog|none"),
                },
                None => self.fail(b"perm answer once|always|deny|never|dialog|none"),
            },
            Some("timeout") => match w.get(1).and_then(|x| x.parse::<u32>().ok()) {
                Some(ms) => super::perm::set_wait(ms),
                None => self.fail(b"perm timeout MS"),
            },
            Some("store") => match w.get(1).copied() {
                None | Some("none") => super::perm::set_store(None),
                Some(f) => super::perm::set_store(Some(f)),
            },
            Some("reload") => {
                policy::reload();
                self.invalidate();
                self.run_passes(app);
            }
            Some("wait") => {
                let ms = w.get(1).and_then(|x| x.parse::<u64>().ok()).unwrap_or(5000);
                let deadline = plat::time_us() + ms * 1000;
                while policy::questions().is_empty() {
                    if plat::time_us() >= deadline {
                        self.fail(b"no permission question was asked");
                        break;
                    }
                    // a worker's question wakes this thread (plat::wake)
                    plat::wait_events(plat::time_us() + 2_000, &mut |e| {
                        plat::evq_push(*e);
                    });
                    self.process_events(app);
                }
                self.invalidate();
                self.process_events(app);
                self.run_passes(app);
            }
            Some("reset") => {
                policy::forget_all();
                self.invalidate();
                self.run_passes(app);
            }
            Some("expect") if w.len() == 2 && matches!(w[1], "open" | "none") => {
                let open = !policy::questions().is_empty();
                if open != (w[1] == "open") {
                    self.fail(if open { b"a question is open" } else { b"no question is open" });
                }
            }
            Some("expect") if w.len() >= 3 => {
                let rule = w[1..w.len() - 1].join(" ");
                let want = w[w.len() - 1];
                let got = policy::decisions().into_iter().find(|d| d.rule == rule).and_then(|d| d.answer);
                let got = got.map_or("unanswered", Answer::word);
                if got != want {
                    self.fail(format!("{} is {}", rule, got).as_bytes());
                }
            }
            _ => self.fail(b"perm answer|timeout|store|reload|wait|reset|expect ..."),
        }
    }

    pub(crate) fn sc_agent(&mut self, app: &mut dyn App, a: &[u8]) {
        let line = String::from_utf8_lossy(a).into_owned();
        let (verb, rest) = line.trim().split_once(' ').unwrap_or((line.trim(), ""));
        match verb {
            "on" | "off" => {
                self.set_agent_enabled(verb == "on");
                self.run_passes(app);
            }
            "call" => {
                let rest = rest.trim();
                let (action, input) = rest.split_once(' ').unwrap_or((rest, "{}"));
                let input = if input.trim().is_empty() { "{}" } else { input.trim() };
                self.sc.agent.calls += 1;
                let req = format!(
                    "{{\"jsonrpc\":\"2.0\",\"id\":{},\"method\":\"actions/execute\",\"params\":{{\"action\":\"{}\",\"input\":{}}}}}",
                    self.sc.agent.calls, action, input
                );
                let mut session = self.sc.agent.session.take().unwrap_or_else(|| agent_core::RpcSession::new(agent_core::Protocol::Json, "script", "1"));
                let reply = self.agent_request(app, &mut session, req.as_bytes());
                self.sc.agent.session = Some(session);
                self.sc.agent.reply = reply;
                self.run_passes(app); // a question the call raised shows now
            }
            "expect" => {
                let want = rest.trim();
                let reply = self.sc.agent.reply.clone().unwrap_or_default();
                let v = agent_core::json::parse(reply.trim().as_bytes()).ok();
                let got = match v.as_ref() {
                    Some(v) if v.get("result").is_some() => String::from("ok"),
                    Some(v) => {
                        let code = v.get("error").and_then(|e| e.get("data")).and_then(|d| d.get("code")).and_then(agent_core::Value::as_str);
                        format!("error {}", code.unwrap_or("?"))
                    }
                    None => String::from("no reply"),
                };
                if got != want {
                    self.fail(format!("{} ({})", got, reply.trim()).as_bytes());
                }
            }
            _ => self.fail(b"agent on|off|call ACTION [JSON]|expect ok|error CODE"),
        }
    }
}
