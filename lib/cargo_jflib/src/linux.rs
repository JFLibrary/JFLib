// Linux tools for the package formats that need them (rpmbuild, flatpak-builder): on Linux this
// machine, on Windows a WSL distribution that has the tool -- JFLIB_WSL_DISTRO, else the first
// one listed that has it (the default one may be docker-desktop's).  A script runs with `sh`.

use std::path::Path;
use std::process::{Command, Output, Stdio};

pub(crate) struct Linux {
    /// the WSL distribution; None: this machine
    wsl: Option<String>,
}

fn quiet(cmd: &mut Command) -> Option<Output> {
    cmd.stdin(Stdio::null()).output().ok()
}

fn has(wsl: Option<&str>, tool: &str) -> bool {
    let check = format!("command -v {tool} >/dev/null 2>&1");
    let out = match wsl {
        Some(d) => quiet(Command::new("wsl.exe").args(["-d", d, "-e", "sh", "-c", &check])),
        None => quiet(Command::new("sh").args(["-c", &check])),
    };
    out.is_some_and(|o| o.status.success())
}

/// the WSL distributions (wsl.exe -l -q writes UTF-16)
fn distributions() -> Vec<String> {
    let Some(out) = quiet(Command::new("wsl.exe").args(["-l", "-q"])) else { return Vec::new() };
    let units: Vec<u16> = out.stdout.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect();
    String::from_utf16_lossy(&units).lines().map(|l| l.trim_matches(|c: char| c == '\u{feff}' || c == '\0' || c.is_whitespace()).to_string())
        .filter(|l| !l.is_empty() && !l.starts_with("docker-desktop")).collect()
}

impl Linux {
    /// a Linux with `tool` -> Err(why there is none)
    pub(crate) fn with(tool: &str) -> Result<Linux, String> {
        if !cfg!(windows) {
            return if has(None, tool) { Ok(Linux { wsl: None }) } else { Err(format!("{tool} is not installed")) };
        }
        let chosen = std::env::var("JFLIB_WSL_DISTRO").ok().filter(|d| !d.is_empty());
        let candidates = chosen.clone().map(|d| vec![d]).unwrap_or_else(distributions);
        if candidates.is_empty() {
            return Err(format!("{tool} needs Linux: no WSL distribution here"));
        }
        candidates.into_iter().find(|d| has(Some(d), tool)).map(|d| Linux { wsl: Some(d) }).ok_or_else(|| match chosen {
            Some(d) => format!("{tool} is not installed in the WSL distribution {d} (JFLIB_WSL_DISTRO)"),
            None => format!("no WSL distribution here has {tool} (JFLIB_WSL_DISTRO names one)"),
        })
    }

    /// a path of this machine as that Linux sees it
    pub(crate) fn path(&self, p: &Path) -> String {
        let s = p.to_string_lossy().replace('\\', "/");
        let s = s.strip_prefix("//?/").unwrap_or(&s).to_string();
        match (&self.wsl, s.as_bytes()) {
            (Some(_), [d, b':', ..]) => format!("/mnt/{}{}", (*d as char).to_ascii_lowercase(), &s[2..]),
            _ => s,
        }
    }

    /// `sh script args..` -> its output (Err: it did not start)
    pub(crate) fn run(&self, script: &Path, args: &[String]) -> Result<Output, String> {
        let mut cmd = match &self.wsl {
            Some(d) => {
                let mut c = Command::new("wsl.exe");
                c.args(["-d", d, "-e", "sh"]);
                c
            }
            None => Command::new("sh"),
        };
        cmd.arg(self.path(script)).args(args).stdin(Stdio::null()).output().map_err(|e| format!("sh: {e}"))
    }

    pub(crate) fn name(&self) -> String {
        self.wsl.as_ref().map(|d| format!("WSL {d}")).unwrap_or_else(|| "this machine".to_string())
    }
}
