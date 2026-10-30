// aaa -- logins at the IOS-like console and the AAA / RADIUS commands: the login gate
// (Username / Password without echo, three tries, our own refusals), "enable" and "disable"
// with the enable secret, the local credentials ("enable secret / password", "username",
// "service password-encryption", "banner motd"), and the AAA configuration ("aaa new-model",
// "radius server", "aaa group server radius", "aaa authentication / authorization /
// accounting", "login authentication" on a line); the running configuration of all of it; the
// show and debug commands.  The RADIUS exchange itself is radiusd.rs (the NAS service)
//
//   Gate                          a login or an "enable" that waits for input; the console's
//                                 prompt and hidden-input flag come from it
//   build_aaa(tree, modes)        the commands in an IOS command tree (ios.rs)
//   Engine::ios_aaa(ni, p, line)  the handlers of all the commands above
//   Engine::console_login(ni)     starts the console login gate when the console line asks for
//                                 it (called at boot and after a logout)
//   Engine::gate_line(ni, line)   a line typed into the gate (cli/mod.rs routes it here)
//   Engine::enable_cmd / disable_cmd   "enable [level]" / "disable [level]"
//   Engine::aaa_result            the NAS reports a RADIUS login's outcome (radiusd.rs)
//   Engine::aaa_sync_method / aaa_servers_of   a local method's decision, a method's servers
//   Engine::ios_aaa_config(ni)    the lines of the running configuration
//
// Texts are our own words; only "Username:" and "Password:" follow the usual prompts.  A plain
// console (no "login" on line con 0, no AAA for it) opens at the user level as it always has.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use base::cmdtree::{ModeId, Parsed, Tree, F_HIDDEN, F_NO};

use super::ios::K;
use super::Mode;
use crate::aaa::{AuthResult, Method, MethodList, RadiusServer, Requester, ServerGroup};
use crate::auth::{Secret, UserAcct};
use crate::aaatrace::{ANote, Where};
use crate::driver::Report;
use crate::kernel::Engine;
use crate::os::Service;
use crate::trace::Note;

/// the stage a login or enable gate is in
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GKind {
    /// asking for the user name (login local / AAA)
    Username,
    /// asking for the password (echo off)
    Password,
    /// "enable": asking for the enable secret (echo off)
    Enable,
}

/// a login or an "enable" that waits for input at the console (or a vty line)
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Gate {
    pub kind: GKind,
    pub via: Requester,
    /// failed tries so far (three end the login)
    pub tries: u8,
    /// the user name collected at the Username stage
    pub user: String,
    /// the privilege level an "enable" asks for
    pub level: u8,
    /// the login method list to run (empty for an enable gate)
    pub methods: Vec<Method>,
    /// waiting for the NAS to answer a RADIUS request (the console is busy)
    pub waiting: bool,
}

impl Gate {
    /// the prompt the console shows
    pub fn prompt(&self) -> String {
        String::from(match self.kind {
            GKind::Username => "Username: ",
            GKind::Password | GKind::Enable => "Password: ",
        })
    }
    /// the typed line is a password (no echo)
    pub fn secret(&self) -> bool {
        self.kind != GKind::Username
    }
}

/// a method list needs a user name (local or RADIUS), not only a line / enable password
fn needs_user(methods: &[Method]) -> bool {
    methods.iter().any(|m| matches!(m, Method::Local | Method::GroupRadius | Method::Group(_)))
}

// ---------------------------------------------------------------- the command tree

/// the commands of logins and AAA in an IOS tree; `m`: user, privileged, global configuration,
/// line mode
pub(crate) fn build_aaa(t: &mut Tree<K>, m: [ModeId; 4]) {
    let [user, prv, conf, line] = m;
    let sg = t.mode("(config-sg-radius)#", "Commands for this server group:", Some(conf), Some("Undo a setting or restore its default"));
    let rs = t.mode("(config-radius-server)#", "Commands for this RADIUS server:", Some(conf), Some("Undo a setting or restore its default"));

    // ---- EXEC: enable / disable with a level, show, test, debug
    for x in [user, prv] {
        t.add(x, "enable <1-15>=level", "Switch to this privilege level", Some(K::Enable), 0);
    }
    t.add(prv, "disable <0-15>=level", "Drop to this privilege level", Some(K::Disable), 0);
    for x in [user, prv] {
        t.add(x, "show users", "Who is logged in at the lines", Some(K::ShowUsers), 0);
        t.add(x, "show aaa", "AAA state", None, 0);
        t.add(x, "show aaa servers", "The RADIUS servers and their counters", Some(K::ShowAaa), 0);
        t.add(x, "show radius", "RADIUS state", None, 0);
        t.add(x, "show radius statistics", "What the device's RADIUS client counted", Some(K::ShowRadiusStats), 0);
    }
    t.add(prv, "test", "Try a function without changing anything", None, 0);
    t.add(prv, "test aaa", "Try AAA", None, 0);
    t.add(prv, "test aaa group", "Against a server group", None, 0);
    t.add(prv, "test aaa group radius", "Against every RADIUS server", None, 0);
    t.add(prv, "test aaa group radius WORD=user", "The user name", None, 0);
    t.add(prv, "test aaa group radius WORD=user WORD=pass", "The password", None, 0);
    t.add(prv, "test aaa group radius WORD=user WORD=pass legacy", "Try it and show the result", Some(K::TestAaa), 0);
    t.add(prv, "debug aaa", "What AAA does while it happens", None, 0);
    t.add(prv, "debug aaa authentication", "Logins and their methods", Some(K::Debug), 0);
    t.add(prv, "debug radius", "RADIUS requests and answers", Some(K::Debug), 0);
    t.add(prv, "undebug aaa", "Stop AAA debug output", None, 0);
    t.add(prv, "undebug aaa authentication", "Logins and their methods", Some(K::Undebug), 0);
    t.add(prv, "undebug radius", "RADIUS requests and answers", Some(K::Undebug), 0);

    // ---- global configuration: credentials
    t.add(conf, "enable", "The password of the privileged level", None, F_NO);
    for (kw, cmd) in [("secret", K::EnableSecret), ("password", K::EnablePassword)] {
        let help = if kw == "secret" { "A hashed enable secret" } else { "A readable enable password" };
        t.add(conf, &format!("enable {}", kw), help, None, 0);
        t.add(conf, &format!("enable {} LINE=pw", kw), "The password", Some(cmd), 0);
        t.add(conf, &format!("enable {} level", kw), "For one privilege level", None, 0);
        t.add(conf, &format!("enable {} level <0-15>=level", kw), "The level", None, 0);
        t.add(conf, &format!("enable {} level <0-15>=level LINE=pw", kw), "The password", Some(cmd), 0);
        // explicit encodings from a saved configuration
        t.add(conf, &format!("enable {} <0-9>=enc WORD=hash", kw), "The stored form", Some(cmd), F_HIDDEN);
        t.add(conf, &format!("enable {} level <0-15>=level <0-9>=enc WORD=hash", kw), "The stored form", Some(cmd), F_HIDDEN);
        t.add_no(conf, &format!("enable {}", kw), cmd);
        t.add_no(conf, &format!("enable {} level <0-15>=level", kw), cmd);
    }
    t.add(conf, "username", "A local user", None, F_NO);
    t.add(conf, "username WORD=name", "The user name", None, 0);
    t.add_no(conf, "username WORD=name", K::Username);
    for (kw, _c) in [("secret", 0), ("password", 0)] {
        t.add(conf, &format!("username WORD=name {}", kw), if kw == "secret" { "A hashed password" } else { "A readable password" }, None, 0);
        t.add(conf, &format!("username WORD=name {} LINE=pw", kw), "The password", Some(K::Username), 0);
        t.add(conf, &format!("username WORD=name {} <0-9>=enc WORD=hash", kw), "The stored form", Some(K::Username), F_HIDDEN);
        t.add(conf, &format!("username WORD=name privilege <0-15>=priv {}", kw), "The password", None, 0);
        t.add(conf, &format!("username WORD=name privilege <0-15>=priv {} LINE=pw", kw), "The password", Some(K::Username), 0);
        t.add(conf, &format!("username WORD=name privilege <0-15>=priv {} <0-9>=enc WORD=hash", kw), "The stored form", Some(K::Username), F_HIDDEN);
    }
    t.add(conf, "username WORD=name privilege", "The privilege level the user gets", None, 0);
    t.add(conf, "username WORD=name privilege <0-15>=priv", "The level (no password)", Some(K::Username), 0);
    t.add(conf, "service", "Device-wide services", None, F_NO);
    t.add(conf, "service password-encryption", "Keep readable passwords obfuscated", Some(K::SvcPwEnc), 0);
    t.add(conf, "banner", "A banner shown at login", None, 0);
    t.add(conf, "banner motd", "The message of the day (delimit the text with a character)", None, 0);
    t.add(conf, "banner motd LINE=text", "A delimiter, the text, the delimiter again", Some(K::Banner), 0);
    t.add_no(conf, "banner motd", K::Banner);

    // ---- AAA
    t.add(conf, "aaa", "Authentication, authorization and accounting", None, F_NO);
    t.add(conf, "aaa new-model", "Turn AAA on", Some(K::AaaNewModel), 0);
    t.add(conf, "aaa authentication", "Who may log in", None, 0);
    t.add(conf, "aaa authentication login", "Logins at the console and the lines", None, 0);
    t.add(conf, "aaa authentication login default", "The default list", None, 0);
    t.add(conf, "aaa authentication login default LINE=methods", "Methods in order (group radius | group NAME | local | line | enable | none)", Some(K::AaaAuthLogin), 0);
    t.add(conf, "aaa authentication login WORD=list", "A named list", None, 0);
    t.add(conf, "aaa authentication login WORD=list LINE=methods", "Methods in order", Some(K::AaaAuthLogin), 0);
    t.add_no(conf, "aaa authentication login default", K::AaaAuthLogin);
    t.add_no(conf, "aaa authentication login WORD=list", K::AaaAuthLogin);
    t.add(conf, "aaa authentication enable", "The enable password check", None, 0);
    t.add(conf, "aaa authentication enable default", "The default list", None, 0);
    t.add(conf, "aaa authentication enable default LINE=methods", "Methods in order (group radius | enable | none)", Some(K::AaaAuthEnable), 0);
    t.add_no(conf, "aaa authentication enable default", K::AaaAuthEnable);
    t.add(conf, "aaa authorization", "What a user may do", None, 0);
    t.add(conf, "aaa authorization exec", "The privilege level at login", None, 0);
    t.add(conf, "aaa authorization exec default", "The default list", None, 0);
    t.add(conf, "aaa authorization exec default LINE=methods", "Methods in order (group radius | local | none)", Some(K::AaaAuthorExec), 0);
    t.add_no(conf, "aaa authorization exec default", K::AaaAuthorExec);
    t.add(conf, "aaa accounting", "What to record", None, 0);
    t.add(conf, "aaa accounting exec", "Records of exec sessions", None, 0);
    t.add(conf, "aaa accounting exec default", "The default list", None, 0);
    t.add(conf, "aaa accounting exec default start-stop=act", "A record at start and at stop", None, 0);
    t.add(conf, "aaa accounting exec default stop-only=act", "A record at stop", None, 0);
    t.add(conf, "aaa accounting exec default start-stop=act LINE=methods", "Methods (group radius | group NAME)", Some(K::AaaAcctExec), 0);
    t.add(conf, "aaa accounting exec default stop-only=act LINE=methods", "Methods (group radius | group NAME)", Some(K::AaaAcctExec), 0);
    t.add_no(conf, "aaa accounting exec default", K::AaaAcctExec);
    t.add(conf, "aaa group", "A group of servers", None, 0);
    t.add(conf, "aaa group server", "A group of servers", None, 0);
    t.add(conf, "aaa group server radius", "A group of RADIUS servers", None, 0);
    t.add(conf, "aaa group server radius WORD=name", "The group's name", Some(K::AaaGroupCmd), 0);
    t.add_no(conf, "aaa group server radius WORD=name", K::AaaGroupCmd);

    // ---- RADIUS servers
    t.add(conf, "radius", "RADIUS settings", None, 0);
    t.add(conf, "radius server", "A RADIUS server", None, 0);
    t.add(conf, "radius server WORD=name", "The server's name", Some(K::RadiusServerCmd), 0);
    t.add_no(conf, "radius server WORD=name", K::RadiusServerCmd);
    t.add(conf, "radius-server", "A RADIUS server (older form)", None, F_NO);
    t.add(conf, "radius-server host A.B.C.D=addr", "The server's address", Some(K::RadiusHost), 0);
    t.add(conf, "radius-server host A.B.C.D=addr auth-port <0-65535>=ap", "The authentication port", Some(K::RadiusHost), 0);
    t.add(conf, "radius-server host A.B.C.D=addr auth-port <0-65535>=ap acct-port <0-65535>=cp", "The accounting port", Some(K::RadiusHost), 0);
    t.add(conf, "radius-server host A.B.C.D=addr key LINE=key", "The shared secret", Some(K::RadiusHost), 0);
    t.add(conf, "radius-server host A.B.C.D=addr auth-port <0-65535>=ap key LINE=key", "The shared secret", Some(K::RadiusHost), 0);
    t.add(conf, "radius-server host A.B.C.D=addr auth-port <0-65535>=ap acct-port <0-65535>=cp key LINE=key", "The shared secret", Some(K::RadiusHost), 0);
    t.add_no(conf, "radius-server host A.B.C.D=addr", K::RadiusHost);

    // the group's submode
    t.add(sg, "server", "A server of this group", None, F_NO);
    t.add(sg, "server name WORD=name", "A server by its name", Some(K::SgServer), 0);
    t.add_no(sg, "server name WORD=name", K::SgServer);
    // the server's submode
    t.add(rs, "address", "The server's address", None, 0);
    t.add(rs, "address ipv4 A.B.C.D=addr", "The IPv4 address", Some(K::RsAddress), 0);
    t.add(rs, "address ipv4 A.B.C.D=addr auth-port <0-65535>=ap", "The authentication port", Some(K::RsAddress), 0);
    t.add(rs, "address ipv4 A.B.C.D=addr auth-port <0-65535>=ap acct-port <0-65535>=cp", "The accounting port", Some(K::RsAddress), 0);
    t.add(rs, "address ipv4 A.B.C.D=addr acct-port <0-65535>=cp", "The accounting port", Some(K::RsAddress), 0);
    t.add(rs, "key", "The shared secret", None, F_NO);
    t.add(rs, "key LINE=key", "The secret", Some(K::RsKey), 0);
    t.add_no(rs, "key", K::RsKey);
    t.add(rs, "timeout", "Seconds to wait for an answer", None, 0);
    t.add(rs, "timeout <1-1000>=sec", "Seconds", Some(K::RsTimeout), 0);
    t.add(rs, "retransmit", "How often to send a request again", None, 0);
    t.add(rs, "retransmit <0-100>=n", "Times", Some(K::RsRetransmit), 0);

    // ---- line: login authentication
    t.add(line, "login local", "Ask for a local user name and password", Some(K::LineLogin), 0);
    t.add(line, "login authentication", "Use an AAA method list", None, 0);
    t.add(line, "login authentication default", "The default list", Some(K::LineLoginAuth), 0);
    t.add(line, "login authentication WORD=list", "A named list", Some(K::LineLoginAuth), 0);
}

// ---------------------------------------------------------------- method parsing

/// the methods of a list from the rest of a line
fn parse_methods(s: &str) -> Result<Vec<Method>, String> {
    let words: Vec<&str> = s.split_whitespace().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < words.len() {
        let (m, n) = Method::parse(&words[i..]).ok_or_else(|| format!("% Unknown authentication method \"{}\"\n", words[i]))?;
        out.push(m);
        i += n;
    }
    if out.is_empty() {
        return Err(String::from("% At least one method is needed\n"));
    }
    Ok(out)
}

// ---------------------------------------------------------------- helpers

impl Engine {
    /// twelve salt bytes from the device's random stream
    fn aaa_salt(&mut self, ni: usize) -> [u8; 12] {
        let mut s = [0u8; 12];
        if let Some(Some(n)) = self.nodes.get_mut(ni) {
            n.rng.fill(&mut s);
        }
        s
    }

    /// a note and a German log line of a login / AAA decision
    pub(crate) fn aaa_note(&mut self, ni: usize, n: ANote) {
        let dev = self.dev_of(ni);
        let at = self.now;
        let sev = if n.is_drop() { 4 } else { 6 };
        let text = n.message().text();
        self.cli.reports.push(Report::Log { dev, at, sev, tag: String::from("aaa"), text });
        self.note(dev, None, Note::Aaa(n));
    }

    fn cfg_mut(&mut self, ni: usize) -> Option<&mut super::DevCfg> {
        self.cdev_mut(ni).map(|c| &mut c.cfg)
    }

    /// a readable password's secret (kind 0, or kind 7 when "service password-encryption" is on)
    fn pw_secret(&self, ni: usize, plain: &str) -> Secret {
        let on = self.cdev(ni).is_some_and(|c| c.cfg.auth.password_encryption);
        if on { Secret::type7(plain) } else { Secret::plain(plain) }
    }

    // ---------------------------------------------------------------- the dispatcher

    pub(crate) fn ios_aaa(&mut self, ni: usize, p: &Parsed<K>, _line: &str) -> Result<(), String> {
        match p.cmd {
            K::EnableSecret | K::EnablePassword => self.cmd_enable_pw(ni, p),
            K::Username => self.cmd_username(ni, p),
            K::SvcPwEnc => {
                if let Some(cfg) = self.cfg_mut(ni) {
                    cfg.auth.set_password_encryption(!p.no);
                }
                Ok(())
            }
            K::Banner => self.cmd_banner(ni, p),
            K::AaaNewModel => {
                if let Some(cfg) = self.cfg_mut(ni) {
                    cfg.aaa.new_model = !p.no;
                }
                Ok(())
            }
            K::AaaAuthLogin => self.cmd_auth_login(ni, p),
            K::AaaAuthEnable => {
                let ms = if p.no { Vec::new() } else { parse_methods(p.text("methods").unwrap_or(""))? };
                if let Some(cfg) = self.cfg_mut(ni) {
                    cfg.aaa.set_enable_list(ms);
                }
                Ok(())
            }
            K::AaaAuthorExec => {
                let ms = if p.no { Vec::new() } else { parse_methods(p.text("methods").unwrap_or(""))? };
                if let Some(cfg) = self.cfg_mut(ni) {
                    cfg.aaa.author_exec = if ms.is_empty() { None } else { Some(MethodList { name: String::new(), methods: ms }) };
                }
                Ok(())
            }
            K::AaaAcctExec => self.cmd_acct(ni, p),
            K::AaaGroupCmd => self.cmd_group(ni, p),
            K::SgServer => self.cmd_sg_server(ni, p),
            K::RadiusServerCmd => self.cmd_radius_server(ni, p),
            K::RadiusHost => self.cmd_radius_host(ni, p),
            K::RsAddress | K::RsKey | K::RsTimeout | K::RsRetransmit => self.cmd_rs(ni, p),
            K::LineLoginAuth => self.cmd_login_auth(ni, p),
            K::ShowUsers => {
                let t = self.show_users(ni);
                self.out(ni, &t);
                Ok(())
            }
            K::ShowAaa => {
                let t = self.show_aaa_servers(ni);
                self.out(ni, &t);
                Ok(())
            }
            K::ShowRadiusStats => {
                let t = self.show_radius_stats(ni);
                self.out(ni, &t);
                Ok(())
            }
            K::TestAaa => self.cmd_test_aaa(ni, p),
            _ => Ok(()),
        }
    }

    fn cmd_enable_pw(&mut self, ni: usize, p: &Parsed<K>) -> Result<(), String> {
        let level = p.num("level").unwrap_or(15) as u8;
        let secret = p.cmd == K::EnableSecret;
        if p.no {
            if let Some(cfg) = self.cfg_mut(ni) {
                if secret {
                    cfg.auth.set_enable_secret(level, None);
                } else {
                    cfg.auth.set_enable_password(level, None);
                }
            }
            return Ok(());
        }
        let s = self.build_secret(ni, p, secret)?;
        if let Some(cfg) = self.cfg_mut(ni) {
            if secret {
                cfg.auth.set_enable_secret(level, Some(s));
            } else {
                cfg.auth.set_enable_password(level, Some(s));
            }
        }
        Ok(())
    }

    /// a Secret from a command: an explicit encoding ("8 $8$..", "7 07..") or a plain password
    /// (`secret`: hash it; else keep it readable, obfuscated when configured)
    fn build_secret(&mut self, ni: usize, p: &Parsed<K>, secret: bool) -> Result<Secret, String> {
        // an explicit encoding: "0 X" means X is plaintext, "7 X" / "8 X" are already encoded
        let plain = match (p.num("enc"), p.text("hash"), p.text("pw")) {
            (Some(0), Some(h), _) => h,
            (Some(enc), Some(h), _) => return Ok(Secret::from_config(Some(enc as u8), h)),
            (_, _, pw) => pw.unwrap_or(""),
        };
        if secret {
            let salt = self.aaa_salt(ni);
            Ok(Secret::pbkdf2(plain, &salt))
        } else {
            Ok(self.pw_secret(ni, plain))
        }
    }

    fn cmd_username(&mut self, ni: usize, p: &Parsed<K>) -> Result<(), String> {
        let name = String::from(p.text("name").unwrap_or(""));
        if name.is_empty() {
            return self.ios_err(ni, "% A user name is needed\n");
        }
        if p.no {
            if let Some(cfg) = self.cfg_mut(ni) {
                cfg.auth.remove_user(&name);
            }
            return Ok(());
        }
        let privilege = p.num("priv").unwrap_or(1).clamp(0, 15) as u8;
        // "username NAME privilege N" without a password: keep an existing secret or make none
        let has_pw = p.has("pw") || p.has("hash");
        let secret_kw = p.has("secret");
        let s = if has_pw {
            self.build_secret(ni, p, secret_kw)?
        } else {
            self.cdev(ni).and_then(|c| c.cfg.auth.user(&name)).map(|u| u.secret.clone()).unwrap_or_else(|| Secret::plain(""))
        };
        if let Some(cfg) = self.cfg_mut(ni) {
            cfg.auth.set_user(UserAcct { name, privilege, secret: s });
        }
        Ok(())
    }

    fn cmd_banner(&mut self, ni: usize, p: &Parsed<K>) -> Result<(), String> {
        if p.no {
            if let Some(cfg) = self.cfg_mut(ni) {
                cfg.motd.clear();
            }
            return Ok(());
        }
        let raw = p.text("text").unwrap_or("").trim_start();
        // the first character is the delimiter; the text runs to its next occurrence
        let mut cs = raw.chars();
        let text = match cs.next() {
            Some(delim) => {
                let rest = cs.as_str();
                match rest.find(delim) {
                    Some(i) => &rest[..i],
                    None => rest,
                }
            }
            None => "",
        };
        if let Some(cfg) = self.cfg_mut(ni) {
            cfg.motd = String::from(text);
        }
        Ok(())
    }

    fn cmd_auth_login(&mut self, ni: usize, p: &Parsed<K>) -> Result<(), String> {
        let name = String::from(p.text("list").unwrap_or(""));
        let ms = if p.no { Vec::new() } else { parse_methods(p.text("methods").unwrap_or(""))? };
        if let Some(cfg) = self.cfg_mut(ni) {
            cfg.aaa.set_login(&name, ms);
        }
        Ok(())
    }

    fn cmd_acct(&mut self, ni: usize, p: &Parsed<K>) -> Result<(), String> {
        if p.no {
            if let Some(cfg) = self.cfg_mut(ni) {
                cfg.aaa.acct_exec = None;
            }
            return Ok(());
        }
        let action = String::from(p.text("act").unwrap_or("start-stop"));
        let ms = parse_methods(p.text("methods").unwrap_or(""))?;
        if let Some(cfg) = self.cfg_mut(ni) {
            cfg.aaa.acct_exec = Some(crate::aaa::AcctExec { name: String::new(), action, methods: ms });
        }
        Ok(())
    }

    fn cmd_group(&mut self, ni: usize, p: &Parsed<K>) -> Result<(), String> {
        let name = String::from(p.text("name").unwrap_or(""));
        if p.no {
            if let Some(cfg) = self.cfg_mut(ni) {
                cfg.aaa.groups.retain(|g| g.name != name);
            }
            return Ok(());
        }
        if let Some(cfg) = self.cfg_mut(ni) {
            if cfg.aaa.group(&name).is_none() {
                cfg.aaa.groups.push(ServerGroup { name: name.clone(), servers: Vec::new() });
            }
        }
        if let Some(c) = self.cdev_mut(ni) {
            c.sg_name = name;
            c.mode = Mode::AaaGroup;
        }
        Ok(())
    }

    fn cmd_sg_server(&mut self, ni: usize, p: &Parsed<K>) -> Result<(), String> {
        let name = String::from(p.text("name").unwrap_or(""));
        let Some(c) = self.cdev_mut(ni) else {
            return Ok(());
        };
        let g = c.sg_name.clone();
        if let Some(group) = c.cfg.aaa.groups.iter_mut().find(|x| x.name == g) {
            if p.no {
                group.servers.retain(|s| *s != name);
            } else if !group.servers.contains(&name) {
                group.servers.push(name);
            }
        }
        Ok(())
    }

    fn cmd_radius_server(&mut self, ni: usize, p: &Parsed<K>) -> Result<(), String> {
        let name = String::from(p.text("name").unwrap_or(""));
        if p.no {
            if let Some(cfg) = self.cfg_mut(ni) {
                cfg.aaa.servers.retain(|s| s.name != name);
            }
            return Ok(());
        }
        if let Some(cfg) = self.cfg_mut(ni) {
            if cfg.aaa.server(&name).is_none() {
                cfg.aaa.servers.push(RadiusServer::new(&name, netpkt::Ipv4::UNSPECIFIED));
            }
        }
        if let Some(c) = self.cdev_mut(ni) {
            c.rs_name = name;
            c.mode = Mode::RadiusSrv;
        }
        Ok(())
    }

    fn cmd_radius_host(&mut self, ni: usize, p: &Parsed<K>) -> Result<(), String> {
        let addr = netpkt::Ipv4(p.ip("addr").unwrap_or([0; 4]));
        let name = format!("{}", addr);
        if p.no {
            if let Some(cfg) = self.cfg_mut(ni) {
                cfg.aaa.servers.retain(|s| s.name != name);
            }
            return Ok(());
        }
        let ap = p.num("ap").map(|n| n as u16);
        let cp = p.num("cp").map(|n| n as u16);
        let key = p.text("key").map(String::from);
        if let Some(cfg) = self.cfg_mut(ni) {
            if cfg.aaa.server(&name).is_none() {
                cfg.aaa.servers.push(RadiusServer::new(&name, addr));
            }
            if let Some(s) = cfg.aaa.server_mut(&name) {
                s.addr = addr;
                if let Some(ap) = ap {
                    s.auth_port = ap;
                }
                if let Some(cp) = cp {
                    s.acct_port = cp;
                }
                if let Some(k) = key {
                    s.key = k;
                }
            }
        }
        Ok(())
    }

    fn cmd_rs(&mut self, ni: usize, p: &Parsed<K>) -> Result<(), String> {
        let Some(c) = self.cdev_mut(ni) else {
            return Ok(());
        };
        let name = c.rs_name.clone();
        let Some(s) = c.cfg.aaa.server_mut(&name) else {
            return Ok(());
        };
        match p.cmd {
            K::RsAddress => {
                s.addr = netpkt::Ipv4(p.ip("addr").unwrap_or([0; 4]));
                if let Some(ap) = p.num("ap") {
                    s.auth_port = ap as u16;
                }
                if let Some(cp) = p.num("cp") {
                    s.acct_port = cp as u16;
                }
            }
            K::RsKey => s.key = if p.no { String::new() } else { String::from(p.text("key").unwrap_or("")) },
            K::RsTimeout => s.timeout = p.num("sec").unwrap_or(5) as u32,
            K::RsRetransmit => s.retransmit = p.num("n").unwrap_or(3) as u32,
            _ => {}
        }
        Ok(())
    }

    fn cmd_login_auth(&mut self, ni: usize, p: &Parsed<K>) -> Result<(), String> {
        let list = if p.has("default") { String::from("default") } else { String::from(p.text("list").unwrap_or("default")) };
        let val = format!("login authentication {}", list);
        if let Some(c) = self.cdev_mut(ni) {
            let i = c.line;
            if let Some(l) = c.cfg.lines.get_mut(i) {
                l.opts.retain(|o| !o.starts_with("login"));
                if !p.no {
                    l.opts.push(val);
                }
            }
        }
        Ok(())
    }

    fn cmd_test_aaa(&mut self, ni: usize, p: &Parsed<K>) -> Result<(), String> {
        let user = String::from(p.text("user").unwrap_or(""));
        let pass = String::from(p.text("pass").unwrap_or(""));
        let servers = self.aaa_servers_of(ni, &Method::GroupRadius);
        if servers.is_empty() {
            return self.ios_err(ni, "% No RADIUS server configured\n");
        }
        self.out(ni, &format!("Trying {} against every RADIUS server ...\n", user));
        self.nas_begin(ni, Requester::Test, &user, &pass, alloc::vec![Method::GroupRadius], false);
        Ok(())
    }
}

// ---------------------------------------------------------------- the login gate

/// a sync method's own word for a note ("lokal", "Leitungspasswort", ...)
fn how_of(m: &Method) -> &'static str {
    match m {
        Method::Local => "lokal",
        Method::Line => "Leitungspasswort",
        Method::Enable => "enable-Passwort",
        Method::None => "ohne",
        _ => "RADIUS",
    }
}

impl Engine {
    /// the login methods a line asks for (None: it opens without a login)
    pub(crate) fn line_login_plan(&self, ni: usize, line_name: &str) -> Option<Vec<Method>> {
        let c = self.cdev(ni)?;
        let line = c.cfg.lines.iter().find(|l| l.name == line_name)?;
        let aaa = &c.cfg.aaa;
        if let Some(opt) = line.opts.iter().find(|o| o.starts_with("login authentication ")) {
            let name = opt.trim_start_matches("login authentication ").trim();
            let name = if name == "default" { "" } else { name };
            return Some(aaa.login_for(name).map(|l| l.methods.clone()).unwrap_or_default());
        }
        if line.opts.iter().any(|o| o == "login local") {
            return Some(alloc::vec![Method::Local]);
        }
        if line.opts.iter().any(|o| o == "login") {
            if aaa.new_model {
                return Some(aaa.login_for("").map(|l| l.methods.clone()).unwrap_or_else(|| alloc::vec![Method::Line]));
            }
            return Some(alloc::vec![Method::Line]);
        }
        None
    }

    /// the password of a line (plain text in the line's "password" option), if any
    fn line_password(&self, ni: usize, via: Requester) -> Option<Secret> {
        let name = match via {
            Requester::Console => String::from("con 0"),
            Requester::Vty(_) => self.vty_line_name(ni, via),
            Requester::Test => return None,
        };
        let c = self.cdev(ni)?;
        let line = c.cfg.lines.iter().find(|l| l.name == name)?;
        line.opts.iter().find_map(|o| o.strip_prefix("password ")).map(Secret::plain)
    }

    /// starts (or restarts) the console login gate when line con 0 asks for a login
    pub(crate) fn console_login(&mut self, ni: usize) {
        let Some(methods) = self.line_login_plan(ni, "con 0") else {
            if let Some(c) = self.cdev_mut(ni) {
                c.gate = None;
            }
            return;
        };
        let motd = self.cdev(ni).map(|c| c.cfg.motd.clone()).unwrap_or_default();
        if !motd.is_empty() {
            self.out(ni, &format!("{}\n", motd));
        }
        let kind = if needs_user(&methods) { GKind::Username } else { GKind::Password };
        if let Some(c) = self.cdev_mut(ni) {
            c.priv_level = 1;
            c.user.clear();
            c.mode = Mode::User;
            c.gate = Some(Gate { kind, via: Requester::Console, tries: 0, user: String::new(), level: 0, methods, waiting: false });
        }
    }

    /// a line typed into the login / enable gate (cli/mod.rs routes it here)
    pub(crate) fn gate_line(&mut self, ni: usize, line: &str) {
        let Some(g) = self.cdev(ni).and_then(|c| c.cli_gate_clone()) else {
            return;
        };
        match g.kind {
            GKind::Username => {
                let u = line.trim();
                if u.is_empty() {
                    return;
                }
                if let Some(c) = self.cdev_mut(ni) {
                    if let Some(gate) = c.gate.as_mut() {
                        gate.user = String::from(u);
                        gate.kind = GKind::Password;
                    }
                }
            }
            GKind::Password => {
                let user = g.user.clone();
                self.login_run(ni, g.via, g.methods.clone(), user, String::from(line));
            }
            GKind::Enable => self.enable_gate_verify(ni, g.via, g.level, line),
        }
    }

    /// runs a login's method list: sync methods decide here, a RADIUS group goes to the NAS
    fn login_run(&mut self, ni: usize, via: Requester, methods: Vec<Method>, user: String, pass: String) {
        for i in 0..methods.len() {
            match &methods[i] {
                Method::GroupRadius | Method::Group(_) => {
                    let servers = self.aaa_servers_of(ni, &methods[i]);
                    if servers.is_empty() {
                        continue;
                    }
                    let want = self.aaa_want_author(ni);
                    self.set_waiting(ni, via, true);
                    self.nas_begin(ni, via, &user, &pass, methods[i..].to_vec(), want);
                    return;
                }
                m => {
                    let how = how_of(m);
                    let r = self.aaa_sync_method(ni, via, m, &user, &pass);
                    self.login_apply(ni, via, user, r, how);
                    return;
                }
            }
        }
        self.login_apply(ni, via, user, AuthResult::NoServer, "RADIUS");
    }

    /// a single sync method's decision (local, line, enable, none)
    pub(crate) fn aaa_sync_method(&mut self, ni: usize, via: Requester, m: &Method, user: &str, pass: &str) -> AuthResult {
        match m {
            Method::Local => match self.cdev(ni).and_then(|c| c.cfg.auth.verify_user(user, pass)) {
                Some(level) => AuthResult::Pass(level),
                None => AuthResult::Fail,
            },
            Method::Line => match self.line_password(ni, via) {
                Some(s) if s.verify(pass) => AuthResult::Pass(1),
                _ => AuthResult::Fail,
            },
            Method::Enable => {
                if self.cdev(ni).is_some_and(|c| c.cfg.auth.verify_enable(15, pass)) {
                    AuthResult::Pass(15)
                } else {
                    AuthResult::Fail
                }
            }
            Method::None => AuthResult::Pass(1),
            _ => AuthResult::NoServer,
        }
    }

    /// the servers a method would ask
    pub(crate) fn aaa_servers_of(&self, ni: usize, m: &Method) -> Vec<RadiusServer> {
        self.cdev(ni).map(|c| c.cfg.aaa.servers_of(m)).unwrap_or_default()
    }

    /// exec authorization takes the privilege level from a RADIUS reply
    fn aaa_want_author(&self, ni: usize) -> bool {
        self.cdev(ni).is_some_and(|c| c.cfg.aaa.author_exec.as_ref().is_some_and(|l| l.methods.iter().any(|m| matches!(m, Method::GroupRadius | Method::Group(_)))))
    }

    fn set_waiting(&mut self, ni: usize, via: Requester, on: bool) {
        match via {
            Requester::Console => {
                if let Some(c) = self.cdev_mut(ni)
                    && let Some(g) = c.gate.as_mut()
                {
                    g.waiting = on;
                }
            }
            Requester::Vty(l) => self.vty_set_waiting(ni, l, on),
            Requester::Test => {}
        }
        self.cli_state(ni);
    }

    /// the NAS reports a RADIUS login's (or enable's) outcome; `how` named the method
    pub(crate) fn aaa_result(&mut self, ni: usize, via: Requester, outcome: AuthResult, how: &'static str) {
        self.set_waiting(ni, via, false);
        match via {
            Requester::Console => {
                let (user, is_enable, level) = match self.cdev(ni).and_then(|c| c.cli_gate_clone()) {
                    Some(g) => (g.user.clone(), g.kind == GKind::Enable, g.level),
                    None => return,
                };
                if is_enable {
                    self.enable_apply(ni, level, outcome);
                } else {
                    self.login_apply(ni, via, user, outcome, how);
                }
            }
            Requester::Vty(l) => self.vty_apply(ni, l, outcome),
            Requester::Test => {
                let msg = match outcome {
                    AuthResult::Pass(l) => format!("User was authenticated (privilege {}).\n", l),
                    AuthResult::Fail => String::from("User was rejected.\n"),
                    _ => String::from("No RADIUS server answered.\n"),
                };
                self.out(ni, &msg);
                self.cli_state(ni);
            }
        }
    }

    /// applies a console login's outcome
    fn login_apply(&mut self, ni: usize, via: Requester, user: String, outcome: AuthResult, how: &str) {
        if let Requester::Vty(l) = via {
            self.vty_apply(ni, l, outcome);
            return;
        }
        match outcome {
            AuthResult::Pass(level) => {
                if let Some(c) = self.cdev_mut(ni) {
                    c.gate = None;
                    c.user = user.clone();
                    c.priv_level = level;
                    c.mode = if level >= 2 { Mode::Priv } else { Mode::User };
                }
                self.aaa_note(ni, ANote::LoginOk { at: Where::Console, user: user.clone(), how: String::from(how), level });
                self.aaa_acct(ni, via, &user, true);
                self.cli_state(ni);
            }
            AuthResult::Fail => {
                self.aaa_note(ni, ANote::LoginBad { at: Where::Console, user });
                self.console_retry(ni);
            }
            _ => {
                self.aaa_note(ni, ANote::NoMethod);
                self.out(ni, "% Authentication failed: no server reachable and no fallback\n\n");
                self.console_login(ni);
                self.cli_state(ni);
            }
        }
    }

    /// a wrong password at the console: a new try, or a fresh login after three
    fn console_retry(&mut self, ni: usize) {
        let tries = self.cdev(ni).and_then(|c| c.gate.as_ref()).map(|g| g.tries + 1).unwrap_or(3);
        if tries >= 3 {
            self.aaa_note(ni, ANote::LoginDenied { at: Where::Console });
            self.out(ni, "% Access denied\n\n");
            self.console_login(ni);
        } else {
            if let Some(c) = self.cdev_mut(ni)
                && let Some(g) = c.gate.as_mut()
            {
                g.tries = tries;
                let needs = needs_user(&g.methods);
                g.kind = if needs { GKind::Username } else { GKind::Password };
                g.user.clear();
            }
            self.out(ni, "% Access denied\n\n");
        }
        self.cli_state(ni);
    }

    // ---------------------------------------------------------------- enable / disable

    /// "enable [level]"
    pub(crate) fn enable_cmd(&mut self, ni: usize, p: &Parsed<K>) -> Result<(), String> {
        let level = p.num("level").unwrap_or(15).clamp(0, 15) as u8;
        let (aaa_enable, local_guard) = match self.cdev(ni) {
            Some(c) => (c.cfg.aaa.new_model && c.cfg.aaa.enable_list().is_some(), c.cfg.auth.enable_guarded(level)),
            None => (false, false),
        };
        if !aaa_enable && !local_guard {
            if let Some(c) = self.cdev_mut(ni) {
                c.priv_level = level;
                c.mode = if level >= 2 { Mode::Priv } else { Mode::User };
            }
            return Ok(());
        }
        let user = self.cdev(ni).map(|c| c.user.clone()).unwrap_or_default();
        if let Some(c) = self.cdev_mut(ni) {
            c.gate = Some(Gate { kind: GKind::Enable, via: Requester::Console, tries: 0, user, level, methods: Vec::new(), waiting: false });
        }
        Ok(())
    }

    /// "disable [level]"
    pub(crate) fn disable_cmd(&mut self, ni: usize, p: &Parsed<K>) -> Result<(), String> {
        let level = p.num("level").unwrap_or(1).clamp(0, 15) as u8;
        if let Some(c) = self.cdev_mut(ni) {
            c.priv_level = level;
            c.mode = if level >= 2 { Mode::Priv } else { Mode::User };
        }
        Ok(())
    }

    /// a password typed at the enable gate
    fn enable_gate_verify(&mut self, ni: usize, via: Requester, level: u8, pass: &str) {
        let aaa_enable = self.cdev(ni).is_some_and(|c| c.cfg.aaa.new_model && c.cfg.aaa.enable_list().is_some());
        if aaa_enable {
            let methods = self.cdev(ni).and_then(|c| c.cfg.aaa.enable_list().cloned()).map(|l| l.methods).unwrap_or_default();
            for i in 0..methods.len() {
                match &methods[i] {
                    Method::GroupRadius | Method::Group(_) => {
                        let servers = self.aaa_servers_of(ni, &methods[i]);
                        if servers.is_empty() {
                            continue;
                        }
                        self.set_waiting(ni, via, true);
                        let user = format!("$enab{}$", level);
                        self.nas_begin(ni, via, &user, pass, methods[i..].to_vec(), false);
                        return;
                    }
                    Method::None => {
                        self.enable_apply(ni, level, AuthResult::Pass(level));
                        return;
                    }
                    _ => {
                        let ok = self.cdev(ni).is_some_and(|c| c.cfg.auth.verify_enable(level, pass));
                        self.enable_apply(ni, level, if ok { AuthResult::Pass(level) } else { AuthResult::Fail });
                        return;
                    }
                }
            }
            self.enable_apply(ni, level, AuthResult::NoServer);
        } else {
            let ok = self.cdev(ni).is_some_and(|c| c.cfg.auth.verify_enable(level, pass));
            self.enable_apply(ni, level, if ok { AuthResult::Pass(level) } else { AuthResult::Fail });
        }
    }

    fn enable_apply(&mut self, ni: usize, level: u8, outcome: AuthResult) {
        match outcome {
            AuthResult::Pass(_) => {
                if let Some(c) = self.cdev_mut(ni) {
                    c.gate = None;
                    c.priv_level = level;
                    c.mode = if level >= 2 { Mode::Priv } else { Mode::User };
                }
                self.aaa_note(ni, ANote::EnableOk { level });
                self.cli_state(ni);
            }
            AuthResult::Fail => {
                self.aaa_note(ni, ANote::EnableBad { level });
                let tries = self.cdev(ni).and_then(|c| c.gate.as_ref()).map(|g| g.tries + 1).unwrap_or(3);
                if tries >= 3 {
                    if let Some(c) = self.cdev_mut(ni) {
                        c.gate = None;
                    }
                    self.out(ni, "% Access denied\n\n");
                } else {
                    if let Some(c) = self.cdev_mut(ni)
                        && let Some(g) = c.gate.as_mut()
                    {
                        g.tries = tries;
                    }
                    self.out(ni, "% Access denied\n");
                }
                self.cli_state(ni);
            }
            _ => {
                if let Some(c) = self.cdev_mut(ni) {
                    c.gate = None;
                }
                self.out(ni, "% Enable denied: no server reachable\n\n");
                self.cli_state(ni);
            }
        }
    }

    // ---------------------------------------------------------------- accounting

    /// an exec accounting record (RFC 2866) when "aaa accounting exec" names a RADIUS group;
    /// sent straight from the device (fire and forget), so it does not need the NAS service
    pub(crate) fn aaa_acct(&mut self, ni: usize, _via: Requester, user: &str, start: bool) {
        use netpkt::radius::{self, Attr, Packet};
        let methods = match self.cdev(ni).and_then(|c| c.cfg.aaa.acct_exec.clone()) {
            Some(a) => a.methods,
            None => return,
        };
        let server = methods.iter().find_map(|m| self.aaa_servers_of(ni, m).into_iter().next());
        let Some(server) = server else {
            return;
        };
        // an id and an accounting session id from the device's random stream
        let (id, sid) = match self.nodes.get_mut(ni).and_then(|n| n.as_mut()) {
            Some(n) => ((n.rng.next_u32() & 0xFF) as u8, format!("{:08x}", n.rng.next_u32())),
            None => return,
        };
        let src = self.source_for(ni, server.addr).unwrap_or(netpkt::Ipv4::UNSPECIFIED);
        let mut p = Packet::new(radius::ACCOUNTING_REQUEST, id, [0; 16]);
        p.push(Attr::int(radius::A_ACCT_STATUS_TYPE, if start { radius::ACCT_START } else { radius::ACCT_STOP }));
        p.push(Attr::text(radius::A_USER_NAME, user));
        p.push(Attr::text(radius::A_ACCT_SESSION_ID, &sid));
        if !src.is_unspecified() {
            p.push(Attr::addr(radius::A_NAS_IP_ADDRESS, src));
        }
        let mut b = p.to_bytes();
        radius::set_accounting_auth(&mut b, server.key.as_bytes(), crypto::md5_parts);
        let sport = self.ephemeral_port(ni, netpkt::IpAddr::V4(src));
        if self.udp_send(ni, Some(netpkt::IpAddr::V4(src)), sport, netpkt::IpAddr::V4(server.addr), server.acct_port, &b, None) {
            self.aaa_note(ni, ANote::AcctSent { to: server.addr, user: String::from(user), start });
        }
    }

    // ---------------------------------------------------------------- show commands

    fn show_users(&self, ni: usize) -> String {
        let mut s = String::from("    Line       User       Privilege\n");
        let c = self.cdev(ni);
        let user = c.map(|c| c.user.clone()).unwrap_or_default();
        let level = c.map(|c| c.priv_level).unwrap_or(1);
        let who = if user.is_empty() { String::from("-") } else { user };
        s.push_str(&format!("*   con 0      {:<10} {}\n", who, level));
        // vty sessions (telnetd.rs)
        s.push_str(&self.vty_users_lines(ni));
        s
    }

    fn show_aaa_servers(&self, ni: usize) -> String {
        let Some(c) = self.cdev(ni) else {
            return String::new();
        };
        let aaa = &c.cfg.aaa;
        if aaa.servers.is_empty() {
            return String::from("No RADIUS servers configured.\n");
        }
        let mut s = String::new();
        for (i, srv) in aaa.servers.iter().enumerate() {
            s.push_str(&format!(
                "RADIUS server {} ({}): auth port {}, acct port {}, timeout {} s, retransmit {}\n",
                i + 1,
                srv.addr,
                srv.auth_port,
                srv.acct_port,
                srv.timeout,
                srv.retransmit
            ));
        }
        s
    }

    fn show_radius_stats(&self, ni: usize) -> String {
        let m = self.service::<crate::radiusd::NasService>(self.dev_of(ni), "radius-nas").map(|n| n.metrics()).unwrap_or_default();
        let get = |name: &str| m.iter().find(|x| x.name == name).map(|x| x.value).unwrap_or(0);
        format!(
            "RADIUS client statistics:\n  Requests sent:   {}\n  Accepts:         {}\n  Rejects:         {}\n  Timeouts:        {}\n  Dropped answers: {}\n",
            get("requests"),
            get("accepts"),
            get("rejects"),
            get("timeouts"),
            get("dropped")
        )
    }

    // ---------------------------------------------------------------- running configuration

    /// the login, AAA and RADIUS lines of the running configuration
    pub(crate) fn ios_aaa_config(&self, ni: usize) -> String {
        let Some(c) = self.cdev(ni) else {
            return String::new();
        };
        let auth = &c.cfg.auth;
        let aaa = &c.cfg.aaa;
        let mut s = String::new();
        if auth.password_encryption {
            s.push_str("service password-encryption\n!\n");
        }
        if aaa.new_model {
            s.push_str("aaa new-model\n!\n");
            for g in &aaa.groups {
                s.push_str(&format!("aaa group server radius {}\n", g.name));
                for sv in &g.servers {
                    s.push_str(&format!(" server name {}\n", sv));
                }
                s.push_str("!\n");
            }
            for l in &aaa.login {
                s.push_str(&format!("aaa authentication login {} {}\n", l.config_name(), methods_words(&l.methods)));
            }
            if let Some(l) = aaa.enable_list() {
                s.push_str(&format!("aaa authentication enable default {}\n", methods_words(&l.methods)));
            }
            if let Some(l) = &aaa.author_exec {
                s.push_str(&format!("aaa authorization exec default {}\n", methods_words(&l.methods)));
            }
            if let Some(a) = &aaa.acct_exec {
                s.push_str(&format!("aaa accounting exec default {} {}\n", a.action, methods_words(&a.methods)));
            }
            s.push_str("!\n");
        }
        for (lvl, sec) in &auth.enable_secret {
            if *lvl == 15 {
                s.push_str(&format!("enable secret {}\n", sec.config_word()));
            } else {
                s.push_str(&format!("enable secret level {} {}\n", lvl, sec.config_word()));
            }
        }
        for (lvl, sec) in &auth.enable_password {
            if *lvl == 15 {
                s.push_str(&format!("enable password {}\n", sec.config_word()));
            } else {
                s.push_str(&format!("enable password level {} {}\n", lvl, sec.config_word()));
            }
        }
        if !auth.enable_secret.is_empty() || !auth.enable_password.is_empty() {
            s.push_str("!\n");
        }
        for u in &auth.users {
            let kw = if u.secret.is_hash() { "secret" } else { "password" };
            if u.privilege == 1 {
                s.push_str(&format!("username {} {} {}\n", u.name, kw, u.secret.config_word()));
            } else {
                s.push_str(&format!("username {} privilege {} {} {}\n", u.name, u.privilege, kw, u.secret.config_word()));
            }
        }
        if !auth.users.is_empty() {
            s.push_str("!\n");
        }
        if !c.cfg.motd.is_empty() {
            let d = if c.cfg.motd.contains('#') { '^' } else { '#' };
            s.push_str(&format!("banner motd {}{}{}\n!\n", d, c.cfg.motd, d));
        }
        for srv in &aaa.servers {
            // a "radius-server host" one keeps its address as its name; write it in that form
            if srv.name == format!("{}", srv.addr) {
                let mut l = format!("radius-server host {}", srv.addr);
                if srv.auth_port != netpkt::radius::PORT_AUTH {
                    l.push_str(&format!(" auth-port {}", srv.auth_port));
                }
                if srv.acct_port != netpkt::radius::PORT_ACCT {
                    l.push_str(&format!(" acct-port {}", srv.acct_port));
                }
                if !srv.key.is_empty() {
                    l.push_str(&format!(" key {}", srv.key));
                }
                s.push_str(&l);
                s.push_str("\n!\n");
            } else {
                s.push_str(&format!("radius server {}\n", srv.name));
                if !srv.addr.is_unspecified() {
                    let mut a = format!(" address ipv4 {}", srv.addr);
                    if srv.auth_port != netpkt::radius::PORT_AUTH {
                        a.push_str(&format!(" auth-port {}", srv.auth_port));
                    }
                    if srv.acct_port != netpkt::radius::PORT_ACCT {
                        a.push_str(&format!(" acct-port {}", srv.acct_port));
                    }
                    s.push_str(&a);
                    s.push('\n');
                }
                if !srv.key.is_empty() {
                    s.push_str(&format!(" key {}\n", srv.key));
                }
                if srv.timeout != 5 {
                    s.push_str(&format!(" timeout {}\n", srv.timeout));
                }
                if srv.retransmit != 3 {
                    s.push_str(&format!(" retransmit {}\n", srv.retransmit));
                }
                s.push_str("!\n");
            }
        }
        s
    }
}

/// the methods of a list as the configuration writes them
fn methods_words(ms: &[Method]) -> String {
    ms.iter().map(|m| m.words()).collect::<Vec<_>>().join(" ")
}

impl super::CliDev {
    /// a clone of the gate (for the flow functions, which mutate the engine)
    pub(crate) fn cli_gate_clone(&self) -> Option<Gate> {
        self.gate.clone()
    }
}
