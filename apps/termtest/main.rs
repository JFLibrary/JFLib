// termtest -- the console widget (ui/term.rs): an IOS-like router console and a fake Linux shell
// on two TermStates, a 10 000-line flood, an ANSI colour table; for tests/*.uis
//
//   build/win/termtest.exe --script apps/termtest/tests/type.uis --verify-dirty
//   build/win/termtest.exe        (a window)
//
// IOS-like (TF_QHELP | TF_CTRL_SHIFT6): Router> / Router# / Router(config)#; enable, disable,
// configure terminal, end, exit, hostname NAME, show version | running-config (paged with
// --More--) | clock | history, ping ADDR (five '!' on a 200 ms timer, Ctrl+Shift+6 aborts);
// `?` lists the commands, Tab completes a unique word, Ctrl+Z leaves the configuration.
// Shell: a coloured prompt; ls, echo, seq N, colors, clear, bell, help, ping [-c N] HOST (a
// line per second, Ctrl+C), sudo CMD (a password without echo), keys (raw keys until q),
// exit; Tab completes commands (several candidates are listed), Ctrl+L clears.
// The font is DejaVu Sans Mono from lib/doc_view/assets (licence: DejaVu-LICENSE.txt there),
// embedded and set with ui.mono_font_set.
#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use ui::*;

const DESC: AppDesc = AppDesc { title: "jfgui Konsole", flags: AF_RESIZABLE, w: 760, h: 520, min_w: 360, min_h: 240, ..AppDesc::NONE };

static MONO: &[u8] = include_bytes!("../../lib/doc_view/assets/DejaVuSansMono.ttf");

const ID_MODE: u32 = wid("mode");
const ID_FLOOD: u32 = wid("flood");
const ID_COLORS: u32 = wid("colors");
const ID_CLEAR: u32 = wid("clear");
const ID_IOS: u32 = 1000; // the consoles take id .. id + 7
const ID_SH: u32 = 1100;
const T_PING: u32 = 1;

const SH_PROMPT: &str = "\x1b[1;32mnutzer@pc\x1b[0m:\x1b[1;34m~\x1b[0m$ ";
const SH_CMDS: [&str; 14] = ["ansi", "bell", "cat", "clear", "colors", "echo", "exit", "help", "keys", "ls", "ping", "seq", "sudo", "uname"];

// IOS-like commands per mode: (name, help); the help texts are our own words
const IOS_USER: [(&str, &str); 4] = [
    ("enable", "Switch to the privileged level"),
    ("exit", "Close this session"),
    ("ping", "Check whether a host answers"),
    ("show", "Display the state of the device"),
];
const IOS_PRIV: [(&str, &str); 5] = [
    ("configure", "Start changing the configuration"),
    ("disable", "Return to the user level"),
    ("exit", "Close this session"),
    ("ping", "Check whether a host answers"),
    ("show", "Display the state of the device"),
];
const IOS_CONF: [(&str, &str); 4] = [
    ("end", "Finish configuring"),
    ("exit", "Leave this configuration level"),
    ("hostname", "Give the device a name"),
    ("interface", "Pick a port to set up"),
];
const IOS_SHOW: [(&str, &str); 4] = [
    ("clock", "Date and time of the device"),
    ("history", "Commands typed in this session"),
    ("running-config", "Configuration in effect now"),
    ("version", "Software, uptime and image file"),
];

struct Ping {
    ios: bool,
    host: String,
    sent: u32,
    count: u32,
}

struct TermTest {
    mode: i32, // 0 IOS, 1 shell
    ios: TermState,
    sh: TermState,
    level: u8, // IOS: 0 user, 1 privileged, 2 configuration
    host: String,
    ping: Option<Ping>,
    sudo: Option<String>,
    focus_pending: bool,
}

impl TermTest {
    fn new() -> TermTest {
        let mut ios = TermState::new(5000);
        ios.write_str("JFNetworker Konsolen-Test (IOS-artig)\n\nConsole ready, press Enter to log in.\n\n");
        let mut sh = TermState::new(20000);
        sh.write_str("JFNetworker Konsolen-Test (Shell) -- \x1b[1mhelp\x1b[0m listet die Befehle.\n");
        sh.set_prompt(SH_PROMPT);
        let mut t = TermTest { mode: 0, ios, sh, level: 0, host: String::from("Router"), ping: None, sudo: None, focus_pending: true };
        t.ios_prompt();
        t
    }

    fn ios_prompt(&mut self) {
        let p = match self.level {
            0 => format!("{}>", self.host),
            1 => format!("{}#", self.host),
            _ => format!("{}(config)#", self.host),
        };
        self.ios.set_prompt(&p);
    }

    fn ios_cmds(&self) -> &'static [(&'static str, &'static str)] {
        match self.level {
            0 => &IOS_USER,
            1 => &IOS_PRIV,
            _ => &IOS_CONF,
        }
    }

    // ------------------------------------------------------------------ IOS

    fn ios_event(&mut self, ui: &mut Ui, ev: TermEv) {
        match ev {
            TermEv::Submit(line) => self.ios_run(ui, &line),
            TermEv::Help(line) => self.ios_help(&line),
            TermEv::Complete { line, caret } => {
                // a unique word is completed, else nothing happens (IOS lists nothing)
                let (head, word) = split_last(&line[..caret]);
                let table = if head.trim().is_empty() { self.ios_cmds() } else { &IOS_SHOW[..] };
                let m: Vec<&str> = table.iter().map(|c| c.0).filter(|c| c.starts_with(word)).collect();
                let new = if m.len() == 1 && !word.is_empty() { format!("{}{} {}", head, m[0], &line[caret..]) } else { line.clone() };
                self.ios.complete_reply(&new, &[] as &[&str]);
            }
            TermEv::Interrupt => {
                if let Some(p) = self.ping.take().filter(|p| p.ios) {
                    self.ios.write_str(&format!("\nSuccess rate is {} percent ({}/5)\n", p.sent * 20, p.sent));
                    self.ios.set_mode(TM_LINE);
                }
            }
            TermEv::Chord(_) => {
                if self.level == 2 {
                    let p = self.ios.prompt.clone();
                    self.ios.write_str(&format!("{}^Z\n", p));
                    self.level = 1;
                    self.ios_prompt();
                }
            }
            _ => {}
        }
    }

    fn ios_run(&mut self, ui: &mut Ui, line: &str) {
        let words: Vec<&str> = line.split_whitespace().collect();
        let Some(&first) = words.first() else { return };
        let m: Vec<&str> = self.ios_cmds().iter().map(|c| c.0).filter(|c| c.starts_with(first)).collect();
        let cmd = match m.len() {
            0 => {
                let col = self.ios.prompt.chars().count() + line.find(first).unwrap_or(0);
                let pad: String = core::iter::repeat_n(' ', col).collect();
                self.ios.write_str(&format!("{}^\n% Invalid input detected at '^' marker.\n\n", pad));
                return;
            }
            1 => m[0],
            _ if m.contains(&first) => first,
            _ => {
                self.ios.write_str(&format!("% Ambiguous command:  \"{}\"\n", line.trim()));
                return;
            }
        };
        match cmd {
            "enable" => self.level = 1,
            "disable" => self.level = 0,
            "configure" => {
                self.ios.write_str("Configuration mode: one command per line; end or Ctrl+Z leaves it.\n");
                self.level = 2;
            }
            "end" => self.level = 1,
            "exit" => self.level = if self.level == 2 { 1 } else { 0 },
            "hostname" => match words.get(1) {
                Some(h) => self.host = String::from(*h),
                None => self.ios.write_str("% Incomplete command.\n\n"),
            },
            "interface" => self.ios.write_str("% Incomplete command.\n\n"),
            "ping" => match words.get(1) {
                Some(h) => {
                    self.ios.write_str(&format!("Ctrl+Shift+6 stops the ping.\nPinging {} with 5 echo requests of 100 bytes, 2 s timeout each:\n", h));
                    self.ping = Some(Ping { ios: true, host: String::from(*h), sent: 0, count: 5 });
                    self.ios.set_mode(TM_BUSY);
                    ui.timer(T_PING, 200);
                }
                None => self.ios.write_str("% Incomplete command.\n\n"),
            },
            "show" => self.ios_show(words.get(1).copied().unwrap_or("")),
            _ => {}
        }
        self.ios_prompt();
    }

    fn ios_show(&mut self, what: &str) {
        let m: Vec<&str> = IOS_SHOW.iter().map(|c| c.0).filter(|c| !what.is_empty() && c.starts_with(what)).collect();
        match m.first().copied() {
            Some("version") => self.ios.write_str(
                "JFNetworker router software 0.1 (termtest)\n\
                 Boot loader 0.1\n\
                 Up for 5 minutes\n\
                 Image: flash:jfn-router-0.1.bin\n\n",
            ),
            Some("running-config") => {
                // long output: paged with --More--
                self.ios.pager(true);
                let mut s = String::from("Collecting the configuration ...\n\n! configuration in effect, 1234 bytes\n!\n");
                s.push_str(&format!("hostname {}\n!\n", self.host));
                for i in 0..12 {
                    s.push_str(&format!("interface GigabitEthernet0/{}\n ip address 10.0.{}.1 255.255.255.0\n duplex auto\n speed auto\n!\n", i, i));
                }
                s.push_str("line con 0\nline vty 0 4\n login\n!\nend\n\n");
                self.ios.write_str(&s);
                self.ios.pager(false);
            }
            Some("clock") => self.ios.write_str("*12:00:00.000 UTC Mon Oct 19 2026\n"),
            Some("history") => {
                let h: Vec<String> = self.ios.history().iter().map(|l| format!("  {}\n", l)).collect();
                self.ios.write_str(&h.concat());
            }
            _ => self.ios.write_str("% Incomplete command.\n\n"),
        }
    }

    // `?`: after a blank the words that may follow, else the commands the word starts
    fn ios_help(&mut self, line: &str) {
        let (head, word) = split_last(line);
        let head = head.trim();
        if !head.is_empty() && !(head.len() >= 2 && "show".starts_with(head)) {
            self.ios.write_str("  <cr>\n\n");
            return;
        }
        let table = if head.is_empty() { self.ios_cmds() } else { &IOS_SHOW[..] };
        if word.is_empty() {
            let mut s = String::new();
            for (c, h) in table {
                s.push_str(&format!("  {:<15}{}\n", c, h));
            }
            s.push('\n');
            self.ios.write_str(&s);
        } else {
            let m: Vec<&str> = table.iter().map(|c| c.0).filter(|c| c.starts_with(word)).collect();
            if m.is_empty() {
                self.ios.write_str("% Unrecognized command\n");
            } else {
                self.ios.write_str(&format!("{}\n\n", m.join("  ")));
            }
        }
    }

    // ------------------------------------------------------------------ shell

    fn sh_event(&mut self, ui: &mut Ui, ev: TermEv) {
        match ev {
            TermEv::Submit(line) => {
                if self.sudo.take().is_some() {
                    self.sh.write_str("nutzer ist nicht in der sudoers-Datei.\n");
                    self.sh.set_mode(TM_LINE);
                    self.sh.set_prompt(SH_PROMPT);
                } else {
                    self.sh_run(ui, &line);
                }
            }
            TermEv::Complete { line, caret } => {
                let (head, word) = split_last(&line[..caret]);
                if head.trim().is_empty() {
                    let m: Vec<&str> = SH_CMDS.iter().copied().filter(|c| c.starts_with(word)).collect();
                    match m.len() {
                        0 => self.sh.complete_reply(&line, &[] as &[&str]),
                        1 => self.sh.complete_reply(&format!("{} ", m[0]), &[] as &[&str]),
                        _ => {
                            let common = common_prefix(&m);
                            self.sh.complete_reply(&common, &m);
                        }
                    }
                } else {
                    self.sh.complete_reply(&line, &[] as &[&str]);
                }
            }
            TermEv::Interrupt => {
                if let Some(p) = self.ping.take().filter(|p| !p.ios) {
                    self.sh.write_str(&format!(
                        "\n--- {} ping statistics ---\n{} packets transmitted, {} received, 0% packet loss, time {}ms\n",
                        p.host,
                        p.sent,
                        p.sent,
                        p.sent.saturating_sub(1) * 1000
                    ));
                    self.sh.set_mode(TM_LINE);
                } else if self.sudo.take().is_some() {
                    self.sh.set_mode(TM_LINE);
                    self.sh.set_prompt(SH_PROMPT);
                }
            }
            TermEv::Chord(c) if c & 0xFFFF == 0x4C => self.sh.write_str("\x1b[H\x1b[2J"),
            TermEv::Eof => self.sh.write_str("exit\nlogout\n"),
            TermEv::Raw(keys) => {
                let mut s = String::new();
                for k in keys {
                    if k == 'q' as u32 {
                        s.push_str("\nRohmodus beendet.\n");
                        self.sh.set_mode(TM_LINE);
                        break;
                    }
                    if k & TK_KEY != 0 {
                        s.push_str(&format!("<{:#x}> ", k & 0xFFFF));
                    } else if k < 32 || k == 127 {
                        s.push_str(&format!("^{} ", char::from_u32((k + 64) & 127).unwrap_or('?')));
                    } else {
                        s.push(char::from_u32(k).unwrap_or('?'));
                        s.push(' ');
                    }
                }
                self.sh.write_str(&s);
            }
            _ => {}
        }
    }

    fn sh_run(&mut self, ui: &mut Ui, line: &str) {
        let words: Vec<&str> = line.split_whitespace().collect();
        let Some(&cmd) = words.first() else { return };
        let args = &words[1..];
        match cmd {
            "help" => self.sh.write_str(&format!("Befehle: {}\n", SH_CMDS.join(" "))),
            "echo" => self.sh.write_str(&format!("{}\n", args.join(" "))),
            "uname" => self.sh.write_str("Linux\n"),
            "ls" => self.sh.write_str("\x1b[1;34mbin\x1b[0m  \x1b[1;34metc\x1b[0m  notizen.txt  \x1b[1;32mstart.sh\x1b[0m  \x1b[4mlink\x1b[0m\n"),
            "cat" => self.sh.write_str("cat: Lesen von der Standardeingabe wird nicht unterstützt\n"),
            "seq" => {
                let n: u32 = args.first().and_then(|a| a.parse().ok()).unwrap_or(10);
                let mut s = String::new();
                for i in 1..=n {
                    s.push_str(&format!("{}\n", i));
                }
                self.sh.write_str(&s);
            }
            "colors" => self.sh.write_str(&color_table()),
            "ansi" => {
                // control characters and escapes the console understands
                self.sh.write_str("abc\rX\n12345\x1b[3D\x1b[K\na\tb\nx\x08y\n\x1b]0;Titel\x07sichtbar\n");
                self.sh.write_str("vorne \x1b[5Gmitte\nGrüße ");
                self.sh.write(&[0xE2, 0x9C]); // a character split over two writes
                self.sh.write(&[0x93, b'\n']);
                self.sh.write_str("\x1b[31;1mrot\x1b[22m \x1b[42mgrün\x1b[49m \x1b[7minvers\x1b[27m \x1b[4;94mblau\x1b[0m\n");
            }
            "clear" => self.sh.write_str("\x1b[H\x1b[2J"),
            "bell" => self.sh.write_str("\x07"),
            "exit" => self.sh.write_str("logout\n"),
            "keys" => {
                self.sh.write_str("Rohmodus: jede Taste wird angezeigt, q beendet.\n");
                self.sh.set_mode(TM_RAW);
            }
            "sudo" => {
                self.sudo = Some(args.join(" "));
                self.sh.set_prompt("[sudo] Passwort für nutzer: ");
                self.sh.set_mode(TM_SECRET);
            }
            "ping" => {
                let (count, host) = match args {
                    ["-c", n, h, ..] => (n.parse().unwrap_or(4), *h),
                    [h, ..] => (u32::MAX, *h),
                    [] => {
                        self.sh.write_str("ping: usage error: Zieladresse erforderlich\n");
                        return;
                    }
                };
                self.sh.write_str(&format!("PING {} ({}) 56(84) bytes of data.\n", host, host));
                self.ping = Some(Ping { ios: false, host: String::from(host), sent: 0, count });
                self.sh.set_mode(TM_BUSY);
                ui.timer(T_PING, 1000);
            }
            _ => self.sh.write_str(&format!("bash: {}: Befehl nicht gefunden\n", cmd)),
        }
    }

    // the ping's timer: the next reply, or the end
    fn ping_tick(&mut self, ui: &mut Ui) {
        let Some(p) = self.ping.as_mut() else { return };
        p.sent += 1;
        if p.ios {
            self.ios.write_str("!");
            if p.sent >= p.count {
                self.ios.write_str(&format!("\nSuccess rate is 100 percent ({}/{}), round-trip min/avg/max = 1/2/4 ms\n", p.sent, p.count));
                self.ios.set_mode(TM_LINE);
                self.ping = None;
            } else {
                ui.timer(T_PING, 200);
            }
        } else {
            self.sh.write_str(&format!("64 bytes from {}: icmp_seq={} ttl=64 time=0.{} ms\n", p.host, p.sent, 40 + p.sent));
            if p.sent >= p.count {
                self.sh.write_str(&format!(
                    "\n--- {} ping statistics ---\n{} packets transmitted, {} received, 0% packet loss\n",
                    p.host, p.sent, p.sent
                ));
                self.sh.set_mode(TM_LINE);
                self.ping = None;
            } else {
                ui.timer(T_PING, 1000);
            }
        }
    }
}

// the line before its last word, the last word
fn split_last(line: &str) -> (&str, &str) {
    let i = line.rfind(' ').map_or(0, |i| i + 1);
    (&line[..i], &line[i..])
}

fn common_prefix(words: &[&str]) -> String {
    let first = words[0];
    let mut n = first.len();
    for w in &words[1..] {
        n = n.min(first.bytes().zip(w.bytes()).take_while(|(a, b)| a == b).count());
    }
    String::from(&first[..n])
}

// the 16 colours, bold, underline, inverse
fn color_table() -> String {
    let mut s = String::new();
    for (row, base) in [(0, 30), (1, 90)] {
        for k in 0..8 {
            s.push_str(&format!("\x1b[{}m F{}{} \x1b[0m", base + k, row, k));
        }
        s.push('\n');
        for k in 0..8 {
            s.push_str(&format!("\x1b[{}m B{}{} \x1b[0m", base + 10 + k, row, k));
        }
        s.push('\n');
    }
    s.push_str("\x1b[1mfett\x1b[0m \x1b[2mblass\x1b[0m \x1b[4munterstrichen\x1b[0m \x1b[7minvers\x1b[0m \x1b[38;5;196m256-rot\x1b[0m\n");
    s
}

impl App for TermTest {
    fn init(&mut self, ui: &mut Ui) -> bool {
        let f = ui.text.load_mem(MONO, 0);
        if f >= 0 {
            ui.mono_font_set(f as u32);
        }
        true
    }

    fn frame(&mut self, ui: &mut Ui) {
        ui.next_size(-1, -1);
        ui.box_begin(0, BOX_V | pad(12), 8);
        ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
        if ui.tabs(ID_MODE, &mut self.mode, &["IOS-artig", "Shell"]) & EV_ACT != 0 {
            self.focus_pending = true;
        }
        ui.spacer(-1);
        ui.next_flags(ST_NOFOCUS);
        if ui.button(ID_FLOOD, "Flut (10 000 Zeilen)") & EV_ACT != 0 {
            let mut s = String::with_capacity(10000 * 48);
            for i in 1..=10000 {
                s.push_str(&format!("Flutzeile {:05} \x1b[3{}m{}\x1b[0m\n", i, i % 7 + 1, "#".repeat(i % 40)));
            }
            self.cur().write_str(&s);
        }
        ui.next_flags(ST_NOFOCUS);
        if ui.button(ID_COLORS, "Farben") & EV_ACT != 0 {
            self.cur().write_str(&color_table());
        }
        ui.next_flags(ST_NOFOCUS);
        if ui.button(ID_CLEAR, "Leeren") & EV_ACT != 0 {
            self.cur().clear();
        }
        ui.box_end();
        let id = if self.mode == 0 { ID_IOS } else { ID_SH };
        if self.focus_pending {
            self.focus_pending = false;
            ui.focus(id);
        }
        ui.next_size(-1, -1);
        if self.mode == 0 {
            for ev in ui.terminal(ID_IOS, &mut self.ios, TF_QHELP | TF_CTRL_SHIFT6) {
                self.ios_event(ui, ev);
            }
        } else {
            for ev in ui.terminal(ID_SH, &mut self.sh, 0) {
                self.sh_event(ui, ev);
            }
        }
        let st = self.cur();
        let (cols, rows) = st.grid();
        let mode = ["Zeile", "Beschäftigt", "Roh", "Geheim"][st.mode.min(3) as usize];
        let status = [format!("Zeilen: {}", st.line_count()), format!("Raster: {}x{}", cols, rows), format!("Modus: {}", mode)];
        ui.box_begin(0, BOX_H, 16);
        for s in &status {
            ui.text(TS_CAPTION | TX_SECONDARY, s);
        }
        ui.box_end();
        ui.box_end();
    }

    fn command(&mut self, ui: &mut Ui, id: u32, arg: u32) -> bool {
        if id == UIC_TIMER && arg == T_PING {
            self.ping_tick(ui);
            return true;
        }
        false
    }
}

impl TermTest {
    fn cur(&mut self) -> &mut TermState {
        if self.mode == 0 { &mut self.ios } else { &mut self.sh }
    }
}

ui::main!(ui::launch(&DESC, TermTest::new(), &style::STYLE));
