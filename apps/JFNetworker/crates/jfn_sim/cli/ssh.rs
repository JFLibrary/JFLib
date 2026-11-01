// ssh -- SSH at the command lines (the protocol is netpkt::ssh, the programs and servers sshd.rs)
//
//   IOS-like   ip domain-name NAME (also ip domain name NAME), crypto key generate rsa [general-keys]
//              [modulus N] (1024 to 4096, 2048 without), crypto key generate ed25519, crypto key
//              zeroize [rsa|ed25519], ip ssh version 2, ip ssh time-out N, ip ssh
//              authentication-retries N and their no-forms; show ip ssh, show ssh, show crypto key
//              mypubkey [rsa|ed25519]; debug ip ssh (D_SSH: the notes of SSH as console lines);
//              ssh [-v 2] [-c CIPHER] -l USER [-p PORT] HOST
//   Linux      ssh [-p PORT] [-l USER] [-i FILE] [-c CIPHER] [-o OPTION=VALUE] [-q] [-T] [-t]
//              [USER@]HOST [COMMAND ...], ssh-keygen [-t ed25519|rsa] [-b BITS] [-f FILE] [-N ""]
//              [-C COMMENT] [-q], ssh-keygen -l -f FILE, ssh-keygen -y -f FILE, ssh-keygen -R HOST,
//              scp [-P PORT] [-i FILE] [-q] [-o OPTION=VALUE] SOURCE TARGET ([USER@]HOST:PATH on
//              one side)
//
// Decisions: a key needs a host name other than the factory's and "ip domain-name", as the classic
// labs do; a new key replaces the old one without a question.  Keys are files of nvram: (they stay
// through a reload and go with the project), not lines of the running configuration, which has
// the "ip domain-name" and "ip ssh" lines with values that are no default.  A key line in a startup
// configuration makes a key only while the device has none, so a reload keeps it.  Key lines
// while the device boots write the file without telling the project: the same seed makes the
// same key on every load.  Help, messages and the layouts of show are our own words.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use base::cmdtree::{ModeId, Parsed, Tree, F_NO};
use netpkt::ssh::{self, KeyPair};

use super::ios::K;
use super::D_SSH;
use crate::kernel::Engine;
use crate::sshd::{home_of, ClientOpts, ScpJob, SshClient, Strict, IOS_ED, IOS_RSA};
use crate::sshtrace::SshNote;
use crate::trace::Note;

/// the commands of SSH in an IOS command tree: user, privileged, global configuration
pub(crate) fn build_ssh(t: &mut Tree<K>, m: [ModeId; 3]) {
    let [user, prv, conf] = m;
    for x in [user, prv] {
        t.add(x, "ssh", "Open an SSH session to a host", None, 0);
        t.add(x, "ssh -l", "The user to log in as", None, 0);
        t.add(x, "ssh -l WORD=user", "The user", None, 0);
        t.add(x, "ssh -l WORD=user WORD=host", "Address or name of the host", Some(K::Ssh), 0);
        t.add(x, "ssh -l WORD=user -p", "The TCP port (22 if not given)", None, 0);
        t.add(x, "ssh -l WORD=user -p <1-65535>=port", "The port", None, 0);
        t.add(x, "ssh -l WORD=user -p <1-65535>=port WORD=host", "Address or name of the host", Some(K::Ssh), 0);
        t.add(x, "ssh -v", "The protocol version", None, 0);
        t.add(x, "ssh -v <2-2>=ver", "Version 2", None, 0);
        t.add(x, "ssh -v <2-2>=ver -l", "The user to log in as", None, 0);
        t.add(x, "ssh -v <2-2>=ver -l WORD=user", "The user", None, 0);
        t.add(x, "ssh -v <2-2>=ver -l WORD=user WORD=host", "Address or name of the host", Some(K::Ssh), 0);
        t.add(x, "ssh -c", "The cipher", None, 0);
        t.add(x, "ssh -c WORD=cipher", "Its name", None, 0);
        t.add(x, "ssh -c WORD=cipher -l", "The user to log in as", None, 0);
        t.add(x, "ssh -c WORD=cipher -l WORD=user", "The user", None, 0);
        t.add(x, "ssh -c WORD=cipher -l WORD=user WORD=host", "Address or name of the host", Some(K::Ssh), 0);
        t.add(x, "show ip ssh", "The SSH server: is it on, its limits and keys", Some(K::ShowIpSsh), 0);
        t.add(x, "show ssh", "The SSH sessions at this device", Some(K::ShowSsh), 0);
        t.add(x, "show crypto", "Keys and what they are for", None, 0);
        t.add(x, "show crypto key", "The keys of this device", None, 0);
        t.add(x, "show crypto key mypubkey", "The public halves", Some(K::ShowCryptoKey), 0);
        t.add(x, "show crypto key mypubkey rsa", "The RSA key", Some(K::ShowCryptoKey), 0);
        t.add(x, "show crypto key mypubkey ed25519", "The Ed25519 key", Some(K::ShowCryptoKey), 0);
    }
    t.add(prv, "debug ip ssh", "What the SSH server and client do while it happens", Some(K::DebugSsh), 0);
    t.add(prv, "undebug ip ssh", "Stop SSH debug output", Some(K::DebugSsh), 0);

    t.add(conf, "ip domain-name", "The domain of this device", None, F_NO);
    t.add(conf, "ip domain-name WORD=dom", "The domain", Some(K::IpDomainName), 0);
    t.add_no(conf, "ip domain-name", K::IpDomainName);
    t.add(conf, "ip domain name", "The domain of this device", None, F_NO);
    t.add(conf, "ip domain name WORD=dom", "The domain", Some(K::IpDomainName), 0);
    t.add_no(conf, "ip domain name", K::IpDomainName);
    t.add(conf, "crypto", "Keys of this device", None, 0);
    t.add(conf, "crypto key", "Make or forget a key", None, 0);
    t.add(conf, "crypto key generate", "Make a key pair (the old one of that kind goes)", None, 0);
    t.add(conf, "crypto key generate rsa", "An RSA key", Some(K::CryptoKeyGen), 0);
    t.add(conf, "crypto key generate rsa general-keys", "An RSA key for every purpose", Some(K::CryptoKeyGen), 0);
    t.add(conf, "crypto key generate rsa modulus", "How long it is", None, 0);
    t.add(conf, "crypto key generate rsa modulus <1024-4096>=bits", "Bits, a multiple of 16", Some(K::CryptoKeyGen), 0);
    t.add(conf, "crypto key generate rsa general-keys modulus", "How long it is", None, 0);
    t.add(conf, "crypto key generate rsa general-keys modulus <1024-4096>=bits", "Bits, a multiple of 16", Some(K::CryptoKeyGen), 0);
    t.add(conf, "crypto key generate ed25519", "An Ed25519 key", Some(K::CryptoKeyGen), 0);
    t.add(conf, "crypto key zeroize", "Forget the keys (the SSH server stops when none is left)", Some(K::CryptoKeyZero), 0);
    t.add(conf, "crypto key zeroize rsa", "The RSA key", Some(K::CryptoKeyZero), 0);
    t.add(conf, "crypto key zeroize ed25519", "The Ed25519 key", Some(K::CryptoKeyZero), 0);
    t.add(conf, "ip ssh", "The SSH server", None, F_NO);
    t.add(conf, "ip ssh version", "Which version it speaks", None, 0);
    t.add(conf, "ip ssh version <1-2>=ver", "Version 2 is all there is", Some(K::IpSsh), 0);
    t.add_no(conf, "ip ssh version", K::IpSsh);
    t.add(conf, "ip ssh time-out", "How long a login may take", None, 0);
    t.add(conf, "ip ssh time-out <1-120>=secs", "Seconds (120 if not given)", Some(K::IpSsh), 0);
    t.add_no(conf, "ip ssh time-out", K::IpSsh);
    t.add(conf, "ip ssh authentication-retries", "How many logins may fail", None, 0);
    t.add(conf, "ip ssh authentication-retries <0-5>=tries", "Tries (3 if not given)", Some(K::IpSsh), 0);
    t.add_no(conf, "ip ssh authentication-retries", K::IpSsh);
}

impl Engine {
    /// the commands of build_ssh
    pub(crate) fn ios_ssh(&mut self, ni: usize, p: &Parsed<K>) -> Result<(), String> {
        match p.cmd {
            K::Ssh => self.ios_ssh_client(ni, p),
            K::IpDomainName => {
                let d = if p.no { String::new() } else { String::from(p.text("dom").unwrap_or("")) };
                if let Some(c) = self.cdev_mut(ni) {
                    c.cfg.ssh.domain = d;
                }
                Ok(())
            }
            K::CryptoKeyGen => self.crypto_key_gen(ni, p),
            K::CryptoKeyZero => {
                let (rsa, ed) = (!p.has("ed25519"), !p.has("rsa"));
                let mut gone = false;
                for (on, path, kind) in [(rsa, IOS_RSA, "RSA"), (ed, IOS_ED, "ed25519")] {
                    if on && self.store(ni).is_some_and(|s| s.exists(path)) {
                        let _ = self.file_del(ni, path, false);
                        let _ = self.file_del(ni, &format!("{}.pub", path), false);
                        self.ssh_note(ni, SshNote::KeyGone { kind: String::from(kind) });
                        gone = true;
                    }
                }
                if !gone {
                    return self.ios_err(ni, "% No such key on this device\n");
                }
                Ok(())
            }
            K::IpSsh => {
                let no = p.no;
                if let Some(v) = p.num("ver") {
                    if v != 2 {
                        return self.ios_err(ni, "% This device speaks SSH version 2 only\n");
                    }
                }
                let (ver, secs, tries, cmd_ver, cmd_secs) = (p.num("ver"), p.num("secs"), p.num("tries"), p.has("version"), p.has("time-out"));
                let Some(c) = self.cdev_mut(ni) else {
                    return Ok(());
                };
                let s = &mut c.cfg.ssh;
                if cmd_ver {
                    s.version2 = !no && ver == Some(2);
                } else if cmd_secs {
                    s.timeout = if no { 120 } else { secs.unwrap_or(120) as u32 };
                } else {
                    s.retries = if no { 3 } else { tries.unwrap_or(3) as u32 };
                }
                Ok(())
            }
            K::ShowIpSsh => {
                let t = self.show_ip_ssh(ni);
                self.out(ni, &t);
                Ok(())
            }
            K::ShowSsh => {
                let t = self.show_ssh(ni);
                self.out(ni, &t);
                Ok(())
            }
            K::ShowCryptoKey => {
                let t = self.show_crypto_key(ni, p);
                self.out(ni, &t);
                Ok(())
            }
            K::DebugSsh => {
                let off = p.has("undebug") || p.no;
                let before = self.cdev(ni).map(|c| c.debug).unwrap_or(0);
                let after = if off { before & !D_SSH } else { before | D_SSH };
                if let Some(c) = self.cdev_mut(ni) {
                    c.debug = after;
                }
                if before == 0 && after != 0 {
                    self.cli.debug += 1;
                } else if before != 0 && after == 0 {
                    self.cli.debug = self.cli.debug.saturating_sub(1);
                }
                self.out(ni, &format!("Debug output for SSH: {}\n", if off { "off" } else { "on" }));
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn ios_ssh_client(&mut self, ni: usize, p: &Parsed<K>) -> Result<(), String> {
        let host = String::from(p.text("host").unwrap_or(""));
        let user = String::from(p.text("user").unwrap_or(""));
        if host.is_empty() || user.is_empty() {
            return self.ios_err(ni, "% A user and a host are needed\n");
        }
        let o = ClientOpts {
            host,
            port: p.num("port").map_or(ssh::PORT, |n| n as u16),
            user,
            cmd: None,
            ios: true,
            home: String::new(),
            identity: None,
            batch: false,
            strict: Strict::No,
            known: None,
            cipher: p.text("cipher").map(String::from),
            quiet: false,
            scp: None,
        };
        self.run_program(ni, "ssh", alloc::boxed::Box::new(SshClient::new(o)), true)
    }

    // ------------------------------------------------------------ keys

    // a key's two files: quietly while the device boots (the same seed makes the same key on
    // every load), else as a change of the project
    fn write_key(&mut self, ni: usize, path: &str, key: &KeyPair, comment: &str) {
        let (private, public) = (key.private_file(comment), format!("{}\n", key.public_line(comment)));
        let pubpath = format!("{}.pub", path);
        if self.booting(ni) {
            let t = self.file_time(ni);
            if let Some(o) = self.os_mut(ni) {
                let _ = o.files.write(path, private.as_bytes(), t);
                let _ = o.files.write(&pubpath, public.as_bytes(), t);
            }
        } else {
            let _ = self.file_put(ni, path, private.as_bytes(), false);
            let _ = self.file_put(ni, &pubpath, public.as_bytes(), false);
        }
    }

    fn crypto_key_gen(&mut self, ni: usize, p: &Parsed<K>) -> Result<(), String> {
        let host = self.host(ni);
        if host == "Router" || host == "Switch" {
            return self.ios_err(ni, &format!("% Please define a hostname other than {}.\n", host));
        }
        let domain = self.cdev(ni).map(|c| c.cfg.ssh.domain.clone()).unwrap_or_default();
        if domain.is_empty() {
            return self.ios_err(ni, "% Please define a domain-name first.\n");
        }
        let ed = p.has("ed25519");
        let bits = p.num("bits").unwrap_or(2048) as usize;
        if !ed && bits % 16 != 0 {
            return self.ios_err(ni, "% The modulus must be a multiple of 16\n");
        }
        let path = if ed { IOS_ED } else { IOS_RSA };
        // a line of a startup configuration keeps the key the device has
        if self.booting(ni) && self.store(ni).is_some_and(|s| s.exists(path)) {
            return Ok(());
        }
        let first = self.store(ni).is_some_and(|s| !s.exists(IOS_RSA) && !s.exists(IOS_ED));
        let Some(Some(n)) = self.nodes.get_mut(ni) else {
            return Ok(());
        };
        let key = if ed {
            let mut seed = [0u8; 32];
            n.rng.fill(&mut seed);
            Some(KeyPair::ed25519(&seed))
        } else {
            KeyPair::rsa(bits, &mut |b: &mut [u8]| n.rng.fill(b))
        };
        let Some(key) = key else {
            return self.ios_err(ni, "% This key could not be made\n");
        };
        let name = format!("{}.{}", host, domain);
        self.write_key(ni, path, &key, &name);
        let kind = if ed { "ed25519" } else { "RSA" };
        self.ssh_note(ni, SshNote::KeyMade { kind: String::from(kind), bits: key.bits(), fingerprint: ssh::fingerprint(&key.blob()) });
        if !self.booting(ni) {
            self.out(ni, &format!("Made a {}-bit {} key for {}.\n", key.bits(), kind, name));
            if first {
                self.ios_log(ni, "%SSH-5-ENABLED: the SSH server is on");
            }
        }
        Ok(())
    }

    // ------------------------------------------------------------ show

    fn show_ip_ssh(&self, ni: usize) -> String {
        let Some(c) = self.cdev(ni) else {
            return String::new();
        };
        let keys = self.ssh_host_keys(ni);
        let on = self.ssh_wanted(ni);
        let mut s = format!("SSH server: {}, version 2.0\n", if on { "on" } else { "off" });
        s.push_str(&format!("Time for a login: {} s, tries: {}\n", c.cfg.ssh.timeout, c.cfg.ssh.retries.max(1)));
        s.push_str("Key exchange: curve25519-sha256\n");
        s.push_str("Ciphers: chacha20-poly1305@openssh.com, aes128-ctr, aes256-ctr\n");
        if keys.is_empty() {
            s.push_str("Host keys: none (crypto key generate makes one)\n");
        } else {
            s.push_str("Host keys:\n");
            for k in keys {
                s.push_str(&format!("  {}, {} bits, {}\n", k.alg(), k.bits(), ssh::fingerprint(&k.blob())));
            }
        }
        s
    }

    fn show_ssh(&self, ni: usize) -> String {
        let Some(c) = self.cdev(ni) else {
            return String::new();
        };
        let mut s = String::from("Line       Cipher                         Host key      Phase    User\n");
        let mut any = false;
        for i in &c.sessions {
            let Some(x) = i.conn.as_ref().and_then(|c| c.ssh.as_ref()) else {
                continue;
            };
            any = true;
            let phase = if x.started { "session" } else if x.authed { "logged in" } else { "login" };
            let user = if x.user.is_empty() { "-" } else { x.user.as_str() };
            let a = x.core.algos();
            s.push_str(&format!("{:<10} {:<30} {:<13} {:<8} {}\n", i.line, a.enc_sc, a.host_key, phase, user));
        }
        if !any {
            return String::from("No SSH sessions.\n");
        }
        s
    }

    fn show_crypto_key(&self, ni: usize, p: &Parsed<K>) -> String {
        let keys = self.ssh_host_keys(ni);
        let host = self.host(ni);
        let domain = self.cdev(ni).map(|c| c.cfg.ssh.domain.clone()).unwrap_or_default();
        let mut s = String::new();
        for k in keys {
            let is_rsa = k.key_type() == "ssh-rsa";
            if (p.has("rsa") && !is_rsa) || (p.has("ed25519") && is_rsa) {
                continue;
            }
            s.push_str(&format!("Key for {}.{}\n  Kind: {}\n  Length: {} bits\n  Fingerprint: {}\n  Public: {}\n", host, domain, k.key_type(), k.bits(), ssh::fingerprint(&k.blob()), k.public_line("")));
        }
        if s.is_empty() {
            return String::from("% No such key on this device\n");
        }
        s
    }

    /// the lines of the running configuration ("ip domain-name" and the "ip ssh" values that are
    /// no default)
    pub(crate) fn ios_ssh_config(&self, ni: usize) -> String {
        let Some(c) = self.cdev(ni) else {
            return String::new();
        };
        let x = &c.cfg.ssh;
        let mut s = String::new();
        if !x.domain.is_empty() {
            s.push_str(&format!("ip domain-name {}\n", x.domain));
        }
        if x.version2 {
            s.push_str("ip ssh version 2\n");
        }
        if x.timeout != 120 {
            s.push_str(&format!("ip ssh time-out {}\n", x.timeout));
        }
        if x.retries != 3 {
            s.push_str(&format!("ip ssh authentication-retries {}\n", x.retries));
        }
        if !s.is_empty() {
            s.push_str("!\n");
        }
        s
    }

    /// "debug ip ssh": a decision of SSH as a console line
    pub(crate) fn ssh_debug(&self, note: &Note) -> String {
        let Note::Ssh(n) = note else {
            return String::new();
        };
        match n {
            SshNote::KeyMade { kind, bits, fingerprint } => format!("SSH: made a {}-bit {} key, {}\n", bits, kind, fingerprint),
            SshNote::KeyGone { kind } => format!("SSH: forgot the {} key\n", kind),
            SshNote::Accept { from, line: Some(l) } => format!("SSH: connection from {} on vty {}\n", from, l),
            SshNote::Accept { from, line: None } => format!("SSH: connection from {}\n", from),
            SshNote::Refused { from, why } => format!("SSH: refused {}: {}\n", from, why),
            SshNote::Keyed { peer, cipher, host_key } => format!("SSH: keys agreed with {}: {}, {}\n", peer, cipher, host_key),
            SshNote::LoginOk { from, user, .. } => format!("SSH: {} from {} logged in\n", user, from),
            SshNote::LoginBad { from, user, .. } => format!("SSH: login of {} from {} refused\n", user, from),
            SshNote::TooManyTries { from, user } => format!("SSH: too many tries of {} from {}\n", user, from),
            SshNote::LoginTimeout { from } => format!("SSH: login from {} took too long\n", from),
            SshNote::Session { from, what } => format!("SSH: session from {}: {}\n", from, what),
            SshNote::Closed { from, .. } => format!("SSH: session from {} closed\n", from),
            SshNote::Broken { peer, why } => format!("SSH: connection with {} broke: {}\n", peer, why),
            SshNote::Connected { to, port } => format!("SSH: connected to {} port {}\n", to, port),
            SshNote::Unreachable { to, port, why } => format!("SSH: {} port {}: {}\n", to, port, why),
            SshNote::ClientLogin { to, user, how } => format!("SSH: logged in as {} at {} ({})\n", user, to, how),
            SshNote::ClientDenied { to, user } => format!("SSH: login as {} at {} given up\n", user, to),
            SshNote::ClientEnd { to, status } => format!("SSH: session with {} ended, status {}\n", to, status),
            _ => String::new(),
        }
    }

    // ------------------------------------------------------------ the Linux shell

    /// ssh
    pub(crate) fn sh_ssh(&mut self, ni: usize, args: &[&str]) -> Result<(), String> {
        const USAGE: &str = "usage: ssh [-p PORT] [-l USER] [-i FILE] [-c CIPHER] [-o OPTION=VALUE] [-q] [-T] [-t] [USER@]HOST [COMMAND ...]\n";
        let mut o = self.ssh_opts(ni);
        let mut i = 0;
        let mut host: Option<String> = None;
        let mut cmd: Vec<String> = Vec::new();
        while i < args.len() {
            let a = args[i];
            if host.is_some() {
                cmd.push(String::from(a));
                i += 1;
                continue;
            }
            let val = |k: &mut usize| -> Option<String> {
                *k += 1;
                args.get(*k).map(|s| String::from(*s))
            };
            match a {
                "-q" => o.quiet = true,
                "-T" | "-t" | "-tt" | "-4" | "-6" | "-v" | "-vv" | "-vvv" | "-A" | "-a" | "-x" | "-X" | "-C" | "-n" => {}
                "-p" => match val(&mut i).and_then(|v| v.parse().ok()) {
                    Some(p) => o.port = p,
                    None => return self.sh_ssh_err(ni, USAGE),
                },
                "-l" => match val(&mut i) {
                    Some(v) => o.user = v,
                    None => return self.sh_ssh_err(ni, USAGE),
                },
                "-i" => match val(&mut i) {
                    Some(v) => o.identity = Some(self.sh_abs(ni, &v)),
                    None => return self.sh_ssh_err(ni, USAGE),
                },
                "-c" => match val(&mut i) {
                    Some(v) => o.cipher = Some(v),
                    None => return self.sh_ssh_err(ni, USAGE),
                },
                "-o" => match val(&mut i) {
                    Some(v) => {
                        if let Err(t) = self.ssh_option(ni, &mut o, &v) {
                            return self.sh_ssh_err(ni, &t);
                        }
                    }
                    None => return self.sh_ssh_err(ni, USAGE),
                },
                a if a.starts_with('-') => return self.sh_ssh_err(ni, USAGE),
                a => host = Some(String::from(a)),
            }
            i += 1;
        }
        let Some(h) = host else {
            return self.sh_ssh_err(ni, USAGE);
        };
        match h.split_once('@') {
            Some((u, x)) => {
                o.user = String::from(u);
                o.host = String::from(x);
            }
            None => o.host = h,
        }
        if !cmd.is_empty() {
            o.cmd = Some(cmd.join(" "));
        }
        self.run_program(ni, "ssh", alloc::boxed::Box::new(SshClient::new(o)), true)
    }

    fn sh_ssh_err(&mut self, ni: usize, text: &str) -> Result<(), String> {
        self.sh_set_code(ni, 255);
        self.sh_err(ni, text)
    }

    // what ssh and scp start with
    fn ssh_opts(&self, ni: usize) -> ClientOpts {
        let sh = self.sh(ni);
        ClientOpts {
            host: String::new(),
            port: ssh::PORT,
            user: String::from(sh.map_or("root", |s| s.var("USER"))),
            cmd: None,
            ios: false,
            home: String::from(sh.map_or("/root", |s| s.home())),
            identity: None,
            batch: false,
            strict: Strict::Ask,
            known: None,
            cipher: None,
            quiet: false,
            scp: None,
        }
    }

    fn ssh_option(&self, ni: usize, o: &mut ClientOpts, v: &str) -> Result<(), String> {
        let Some((k, x)) = v.split_once('=') else {
            return Err(format!("ssh: this option needs a value: {}\n", v));
        };
        match k.to_ascii_lowercase().as_str() {
            "stricthostkeychecking" => {
                o.strict = match x {
                    "yes" => Strict::Yes,
                    "no" | "off" => Strict::No,
                    "accept-new" => Strict::AcceptNew,
                    "ask" => Strict::Ask,
                    _ => return Err(format!("ssh: StrictHostKeyChecking takes yes, no, accept-new or ask, not {}\n", x)),
                }
            }
            "batchmode" => o.batch = x == "yes",
            "userknownhostsfile" => o.known = Some(self.sh_abs(ni, x)),
            "port" => o.port = x.parse().map_err(|_| format!("ssh: not a port: {}\n", x))?,
            "user" => o.user = String::from(x),
            "identityfile" => o.identity = Some(self.sh_abs(ni, x)),
            "ciphers" => o.cipher = Some(String::from(x)),
            "loglevel" | "connecttimeout" | "serveraliveinterval" | "pubkeyauthentication" | "preferredauthentications" | "passwordauthentication" => {}
            _ => return Err(format!("ssh: this option is not known here: {}\n", k)),
        }
        Ok(())
    }

    /// scp
    pub(crate) fn sh_scp(&mut self, ni: usize, args: &[&str]) -> Result<(), String> {
        const USAGE: &str = "usage: scp [-P PORT] [-i FILE] [-q] [-o OPTION=VALUE] SOURCE TARGET (one of them [USER@]HOST:PATH)\n";
        let mut o = self.ssh_opts(ni);
        let mut pos: Vec<String> = Vec::new();
        let mut i = 0;
        while i < args.len() {
            match args[i] {
                "-q" => o.quiet = true,
                "-O" | "-p" | "-v" | "-C" | "-4" | "-6" => {}
                "-P" => {
                    i += 1;
                    match args.get(i).and_then(|v| v.parse().ok()) {
                        Some(p) => o.port = p,
                        None => return self.sh_ssh_err(ni, USAGE),
                    }
                }
                "-i" => {
                    i += 1;
                    match args.get(i) {
                        Some(v) => o.identity = Some(self.sh_abs(ni, v)),
                        None => return self.sh_ssh_err(ni, USAGE),
                    }
                }
                "-o" => {
                    i += 1;
                    let Some(v) = args.get(i) else {
                        return self.sh_ssh_err(ni, USAGE);
                    };
                    if let Err(t) = self.ssh_option(ni, &mut o, v) {
                        return self.sh_ssh_err(ni, &t);
                    }
                }
                a if a.starts_with('-') => return self.sh_ssh_err(ni, USAGE),
                a => pos.push(String::from(a)),
            }
            i += 1;
        }
        let [src, dst] = pos.as_slice() else {
            return self.sh_ssh_err(ni, USAGE);
        };
        // a remote side has a colon before any slash
        let remote = |s: &str| -> Option<(String, String)> {
            let at = s.find(':')?;
            if s[..at].contains('/') {
                return None;
            }
            Some((String::from(&s[..at]), String::from(&s[at + 1..])))
        };
        let (who, local, path, upload) = match (remote(src), remote(dst)) {
            (None, Some((who, path))) => (who, self.sh_abs(ni, src), path, true),
            (Some((who, path)), None) => (who, self.sh_abs(ni, dst), path, false),
            _ => return self.sh_ssh_err(ni, USAGE),
        };
        match who.split_once('@') {
            Some((u, h)) => {
                o.user = String::from(u);
                o.host = String::from(h);
            }
            None => o.host = who,
        }
        if upload && self.store(ni).is_none_or(|s| !s.exists(&local)) {
            return self.sh_ssh_err(ni, &format!("scp: {}: No such file or directory\n", local));
        }
        let path = if path.is_empty() { String::from(".") } else { path };
        let path = if path.starts_with('/') { path } else if path == "." { home_of(&o.user) } else { format!("{}/{}", home_of(&o.user), path) };
        o.scp = Some(ScpJob::new(local, path, upload));
        self.run_program(ni, "scp", alloc::boxed::Box::new(SshClient::new(o)), true)
    }

    /// ssh-keygen
    pub(crate) fn sh_ssh_keygen(&mut self, ni: usize, args: &[&str]) -> Result<(), String> {
        const USAGE: &str = "usage: ssh-keygen [-t ed25519|rsa] [-b BITS] [-f FILE] [-N \"\"] [-C COMMENT] [-q]\n       ssh-keygen -l -f FILE | -y -f FILE | -R HOST [-f FILE]\n";
        let (mut kind, mut bits, mut file, mut comment, mut quiet, mut list, mut print, mut forget) = (String::from("ed25519"), 3072usize, None, None, false, false, false, None);
        let mut i = 0;
        while i < args.len() {
            let a = args[i];
            let mut next = || {
                i += 1;
                args.get(i).copied()
            };
            match a {
                "-q" => quiet = true,
                "-l" => list = true,
                "-y" => print = true,
                "-t" => match next() {
                    Some(v) => kind = String::from(v),
                    None => return self.sh_err(ni, USAGE),
                },
                "-b" => match next().and_then(|v| v.parse().ok()) {
                    Some(v) => bits = v,
                    None => return self.sh_err(ni, USAGE),
                },
                "-f" => match next() {
                    Some(v) => file = Some(String::from(v)),
                    None => return self.sh_err(ni, USAGE),
                },
                "-C" => match next() {
                    Some(v) => comment = Some(String::from(v)),
                    None => return self.sh_err(ni, USAGE),
                },
                "-N" => match next() {
                    Some("") => {}
                    Some(_) => return self.sh_err(ni, "ssh-keygen: key files with a passphrase are not made here\n"),
                    None => return self.sh_err(ni, USAGE),
                },
                "-R" => match next() {
                    Some(v) => forget = Some(String::from(v)),
                    None => return self.sh_err(ni, USAGE),
                },
                _ => return self.sh_err(ni, USAGE),
            }
            i += 1;
        }
        let home = String::from(self.sh(ni).map_or("/root", |s| s.home()));
        if let Some(h) = forget {
            return self.keygen_forget(ni, &h, file.map(|f| self.sh_abs(ni, &f)).unwrap_or_else(|| format!("{}/.ssh/known_hosts", home)));
        }
        if list || print {
            let Some(f) = file else {
                return self.sh_err(ni, USAGE);
            };
            return self.keygen_show(ni, &self.sh_abs(ni, &f), list);
        }
        let default = match kind.as_str() {
            "ed25519" => "id_ed25519",
            "rsa" => "id_rsa",
            _ => return self.sh_err(ni, &format!("ssh-keygen: no keys of the kind {} here (ed25519, rsa)\n", kind)),
        };
        let path = match file {
            Some(f) => self.sh_abs(ni, &f),
            None => format!("{}/.ssh/{}", home, default),
        };
        if self.store(ni).is_some_and(|s| s.exists(&path)) {
            return self.sh_err(ni, &format!("ssh-keygen: {} is there already; remove it first\n", path));
        }
        let comment = comment.unwrap_or_else(|| format!("{}@{}", self.sh(ni).map_or("root", |s| s.var("USER")), self.host(ni)));
        let Some(Some(n)) = self.nodes.get_mut(ni) else {
            return Ok(());
        };
        let key = if kind == "ed25519" {
            let mut seed = [0u8; 32];
            n.rng.fill(&mut seed);
            Some(KeyPair::ed25519(&seed))
        } else {
            KeyPair::rsa(bits, &mut |b: &mut [u8]| n.rng.fill(b))
        };
        let Some(key) = key else {
            return self.sh_err(ni, "ssh-keygen: a key of that length cannot be made (1024 to 8192 bits, a multiple of 16)\n");
        };
        if let Some(dir) = path.rsplit_once('/').map(|x| x.0).filter(|d| !d.is_empty()) {
            let _ = self.file_mkdir(ni, dir, true);
        }
        let pubpath = format!("{}.pub", path);
        if let Err(e) = self.file_put(ni, &path, key.private_file(&comment).as_bytes(), false) {
            return self.sh_err(ni, &format!("ssh-keygen: {}: {}\n", path, e.errno_text()));
        }
        let _ = self.file_put(ni, &pubpath, format!("{}\n", key.public_line(&comment)).as_bytes(), false);
        let fp = ssh::fingerprint(&key.blob());
        let dev = self.dev_of(ni);
        self.note(dev, None, Note::Ssh(SshNote::KeyMade { kind: kind.clone(), bits: key.bits(), fingerprint: fp.clone() }));
        if !quiet {
            self.out(ni, &format!("Made a {} key pair.\nSecret key: {}\nPublic key: {}\nFingerprint: {} {}\n", kind, path, pubpath, fp, comment));
        }
        Ok(())
    }

    // ssh-keygen -R HOST: the lines of a host go from known_hosts
    fn keygen_forget(&mut self, ni: usize, host: &str, path: String) -> Result<(), String> {
        let Some(text) = self.store(ni).and_then(|s| s.read(&path).ok()).map(|b| String::from_utf8_lossy(b).into_owned()) else {
            return self.sh_err(ni, &format!("ssh-keygen: {}: No such file or directory\n", path));
        };
        let mut keep = String::new();
        let mut gone = 0;
        for l in text.lines() {
            if l.split_whitespace().next().is_some_and(|names| names.split(',').any(|n| n == host)) {
                gone += 1;
            } else {
                keep.push_str(l);
                keep.push('\n');
            }
        }
        if gone == 0 {
            self.out(ni, &format!("{} is not in {}.\n", host, path));
            return Ok(());
        }
        let _ = self.file_put(ni, &path, keep.as_bytes(), false);
        self.out(ni, &format!("Forgot {} in {} ({} line{}).\n", host, path, gone, if gone == 1 { "" } else { "s" }));
        Ok(())
    }

    // ssh-keygen -l -f FILE (the bits and the fingerprint), -y -f FILE (the public line)
    fn keygen_show(&mut self, ni: usize, path: &str, list: bool) -> Result<(), String> {
        let Some(text) = self.store(ni).and_then(|s| s.read(path).ok()).map(|b| String::from_utf8_lossy(b).into_owned()) else {
            return self.sh_err(ni, &format!("ssh-keygen: {}: No such file or directory\n", path));
        };
        let line = match KeyPair::parse_private(&text) {
            Some((k, c)) => k.public_line(&c),
            None => String::from(text.trim()),
        };
        let Some(p) = ssh::parse_public(&line) else {
            return self.sh_err(ni, &format!("ssh-keygen: {} is no key file\n", path));
        };
        if list {
            self.out(ni, &format!("{} {} {} ({})\n", ssh::blob_bits(&p.blob), ssh::fingerprint(&p.blob), p.comment, p.key_type));
        } else {
            self.out(ni, &format!("{}\n", line));
        }
        Ok(())
    }
}
