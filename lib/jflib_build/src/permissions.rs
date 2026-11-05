// The syntax of PERMISSIONS: the rules of parse in lib/plat/policy.rs, as perm_check in
// build_rs.py checks them, with the same messages (PERM_CASES there are the tests below).

const KEYS_ARGS: [&str; 7] = ["fs.read", "fs.write", "process", "plugins", "url", "net.connect", "net.listen"];
const KEYS_FLAG: [&str; 9] =
    ["fs.user", "print", "secrets", "autostart", "audio.output", "input.global", "screen.capture", "location", "sensors"];
const KEYS_AGENT: [&str; 2] = ["agent.confirm", "agent.deny"];
const KEYS_SANDBOX: [&str; 1] = ["sandbox"];
const KEYS_PACKAGE: [&str; 1] = ["recent"];
const VARS: [&str; 16] = [
    "CONFIG", "TEMP", "HOME", "EXE", "APPIMAGE", "FONTS", "APPCONFIG", "APPDATA", "APPCACHE", "APPLOG", "DESKTOP", "DOCUMENTS",
    "DOWNLOADS", "PICTURES", "MUSIC", "VIDEOS",
];

/// what `ask` may ask for: the rules with arguments and the flags but fs.user
fn askable() -> Vec<&'static str> {
    KEYS_ARGS.iter().chain(&KEYS_FLAG[1..]).copied().collect()
}

/// a line -> (words, reason): the reason is a "quoted text" at the end, '#' outside it starts a
/// comment (split_line in policy.rs)
fn split(ln: &str) -> Result<(Vec<&str>, Option<&str>), &'static str> {
    let cut = [ln.find('#'), ln.find('"')].into_iter().flatten().min();
    let Some(cut) = cut.filter(|&c| ln.as_bytes()[c] == b'"') else {
        return Ok((ln[..cut.unwrap_or(ln.len())].split_whitespace().collect(), None));
    };
    let rest = &ln[cut + 1..];
    let end = rest.find('"').ok_or("the reason has no closing quote")?;
    let after = rest[end + 1..].trim_start();
    if !after.is_empty() && !after.starts_with('#') {
        return Err("nothing but a comment may follow the reason");
    }
    Ok((ln[..cut].split_whitespace().collect(), Some(&rest[..end])))
}

/// None when a is a path pattern a manifest takes, else the reason
fn pattern_error(a: &str) -> Option<String> {
    if a == "**" {
        return None;
    }
    if let Some(v) = a.strip_prefix('$') {
        let var = v.split(['/', '\\']).next().unwrap_or("");
        let list: Vec<String> = VARS.iter().map(|v| format!("${v}")).collect();
        return (!VARS.contains(&var)).then(|| format!("unknown variable ${var} ({})", list.join(", ")));
    }
    let b = a.as_bytes();
    let drive = b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b[2] == b'/' || b[2] == b'\\');
    if drive || a.starts_with("//") || a.starts_with("\\\\") || a.starts_with('/') {
        return None;
    }
    Some(format!("{a} is not absolute (start it with /, X:/ or a variable)"))
}

fn digits(s: &str) -> Option<u32> {
    (!s.is_empty() && s.bytes().all(|c| c.is_ascii_digit())).then(|| s.parse::<u64>().map_or(u32::MAX, |n| n.min(u32::MAX as u64) as u32))
}

/// None when a is the HOST:PORT of net.connect / net.listen (net_rule in policy.rs)
fn net_error(a: &str, listen: bool) -> Option<String> {
    let bad = Some(format!("{a} is not HOST:PORT (example.org:443, *.example.org:*, 10.0.0.0/8:22, [::1]:8080, *:*)"));
    let Some((host, port)) = a.rsplit_once(':') else { return bad };
    let (lo, hi) = if port == "*" {
        (Some(0), Some(65535))
    } else {
        let mut parts = port.split('-');
        let (lo, hi, more) = (parts.next(), parts.next(), parts.next());
        if more.is_some() {
            return bad;
        }
        let lo = lo.and_then(digits);
        (lo, hi.map_or(lo, digits))
    };
    match (lo, hi) {
        (Some(l), Some(h)) if l <= 65535 && h <= 65535 && l <= h => {}
        _ => return bad,
    }
    if host == "*" || host.eq_ignore_ascii_case("localhost") {
        return None;
    }
    let (h, bits) = match host.rsplit_once('/') {
        Some((h, bits)) => (h, Some(bits)),
        None => (host, None),
    };
    let bits = match bits {
        Some(b) => match digits(b) {
            Some(n) => Some(n),
            None => return bad,
        },
        None => None,
    };
    let bracketed = h.len() >= 2 && h.starts_with('[') && h.ends_with(']');
    let bare = if bracketed { &h[1..h.len() - 1] } else { h };
    if !bracketed && bare.contains(':') {
        return Some(format!("{a}: an IPv6 address needs brackets ([::1]:80)"));
    }
    if let Ok(ip) = bare.parse::<std::net::IpAddr>() {
        let max = if ip.is_ipv4() { 32 } else { 128 };
        return (ip.is_ipv4() == bracketed || bits.is_some_and(|b| b > max)).then(|| bad.unwrap());
    }
    if bracketed || bits.is_some() {
        return bad;
    }
    if listen {
        return Some(format!("{a}: net.listen takes an address, a block, localhost or *"));
    }
    let labels = bare.trim_end_matches('.').to_ascii_lowercase();
    let labels = labels.strip_prefix("*.").unwrap_or(&labels);
    let ok = !labels.is_empty()
        && labels.split('.').all(|x| !x.is_empty() && x.bytes().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_' || c == b'-'));
    (!ok).then(|| bad.unwrap())
}

/// the first bad line of a manifest -> (line number, message)
pub(crate) fn check(text: &str) -> Result<(), (usize, String)> {
    for (i, ln) in text.lines().enumerate() {
        line(ln).map_err(|e| (i + 1, e))?;
    }
    Ok(())
}

fn line(ln: &str) -> Result<(), String> {
    let (mut words, reason) = split(ln)?;
    if words.first() == Some(&"ask") {
        words.remove(0);
        let Some(&key) = words.first() else {
            return Err("ask needs a permission (ask fs.read $DOCUMENTS/**)".into());
        };
        if !askable().contains(&key) {
            return Err(format!("{key} cannot be asked for ({})", askable().join(", ")));
        }
        if let Some(r) = reason {
            if r.len() > 200 || r.chars().any(|c| (c as u32) < 0x20) {
                return Err("a reason has at most 200 bytes and no control characters".into());
            }
        }
    } else if reason.is_some() {
        return Err("a reason belongs to an ask line".into());
    }
    let Some((&key, args)) = words.split_first() else { return Ok(()) };
    if KEYS_AGENT.contains(&key) {
        if args.is_empty() {
            return Err(format!("{key} needs at least one action name (document.save, document.*, *)"));
        }
        for a in args {
            let ok = (1..=128).contains(&a.len())
                && a.bytes().all(|c| c.is_ascii_alphanumeric() || b"._*?-".contains(&c));
            if !ok {
                return Err(format!("{a} is not an action name pattern (letters, digits, . _ - and * ?)"));
            }
        }
    } else if KEYS_SANDBOX.contains(&key) {
        if !(args.is_empty() || args == ["strict"]) {
            return Err(format!("{key} takes nothing or 'strict'"));
        }
    } else if KEYS_PACKAGE.contains(&key) || KEYS_FLAG.contains(&key) {
        if !args.is_empty() {
            return Err(format!("{key} takes no arguments"));
        }
    } else if KEYS_ARGS.contains(&key) {
        if args.is_empty() {
            return Err(format!("{key} needs at least one argument"));
        }
        for a in args {
            let err = match key {
                "url" => (*a != "*" && !(!a.is_empty() && a.bytes().all(|c| c.is_ascii_alphanumeric() || b"+.-".contains(&c))))
                    .then(|| format!("{a} is not a URL scheme (https, mailto, ... or *)")),
                "net.connect" | "net.listen" => net_error(a, key == "net.listen"),
                "process" if !a.contains(['/', '\\', '$']) => None, // a program name looked up in PATH
                _ => pattern_error(a),
            };
            if let Some(e) = err {
                return Err(e);
            }
        }
    } else {
        let known: Vec<&str> = KEYS_ARGS[..2]
            .iter()
            .chain(&KEYS_FLAG[..1])
            .chain(&KEYS_ARGS[2..])
            .chain(&KEYS_FLAG[1..])
            .chain(&KEYS_SANDBOX)
            .chain(&KEYS_PACKAGE)
            .chain(&["ask"])
            .chain(&KEYS_AGENT)
            .copied()
            .collect();
        return Err(format!("unknown permission {key} ({})", known.join(", ")));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// PERM_CASES of build_rs.py: the manifest and the error it must give (None: valid)
    const CASES: &[(&str, Option<&str>)] = &[
        ("ask fs.read $DOCUMENTS/** \"Briefe # öffnen\"  # a comment\nask net.connect *:443\nask print\nagent.confirm doc.* ui.type\nagent.deny *.wipe", None),
        ("ask fs.user", Some("fs.user cannot be asked for")),
        ("fs.user\nrecent", None),
        ("recent all", Some("recent takes no arguments")),
        ("ask recent", Some("recent cannot be asked for")),
        ("print\nask", Some(":2: ask needs a permission")),
        ("fs.read /x \"why\"", Some("a reason belongs to an ask line")),
        ("ask url mailto \"open", Some("no closing quote")),
        ("ask url mailto \"a\" b", Some("nothing but a comment may follow")),
        ("ask agent.deny doc.x", Some("agent.deny cannot be asked for")),
        ("agent.confirm doc/x", Some("is not an action name pattern")),
        ("agent.deny", Some("needs at least one action name")),
        ("sandbox\nsandbox strict", None),
        ("sandbox loose", Some("sandbox takes nothing or 'strict'")),
        ("ask print \"a\x07b\"", Some("no control characters")),
        ("fs.read $PLUGIN/x", Some("unknown variable $PLUGIN")),
        ("audio.output\nask audio.output \"Klänge\"", None),
        ("audio.output on", Some("audio.output takes no arguments")),
        ("ask audio.input", Some("audio.input cannot be asked for")),
    ];

    #[test]
    fn the_cases_of_build_rs() {
        for (text, want) in CASES {
            let got = check(text).err().map(|(line, e)| format!("case:{line}: {e}"));
            match (want, &got) {
                (None, None) => {}
                (Some(w), Some(g)) if g.contains(w) => {}
                _ => panic!("{text:?} gives {got:?}, not {want:?}"),
            }
        }
    }

    #[test]
    fn hosts_and_patterns() {
        for ok in ["example.org:443", "*.example.org:*", "10.0.0.0/8:22", "[::1]:8080", "*:*", "localhost:1-1024", "a_b.de.:80"] {
            assert_eq!(net_error(ok, false), None, "{ok}");
        }
        for bad in ["example.org", "x:70000", "x:5-3", "::1:80", "[10.0.0.1]:1", "10.0.0.0/33:1", "ex ample.org:1", "x:1-2-3", "x:"] {
            assert!(net_error(bad, false).is_some(), "{bad}");
        }
        assert!(net_error("example.org:80", true).unwrap().contains("net.listen takes"));
        assert_eq!(pattern_error("C:\\Users\\**"), None);
        assert_eq!(pattern_error("$HOME/x"), None);
        assert!(pattern_error("rel/x").unwrap().contains("is not absolute"));
        assert!(check("process notepad.exe\nprocess $EXE/x\nurl https *\nfs.write //server/share/**").is_ok());
        assert!(check("frobnicate").unwrap_err().1.starts_with("unknown permission frobnicate (fs.read, fs.write, fs.user, process,"));
    }
}
