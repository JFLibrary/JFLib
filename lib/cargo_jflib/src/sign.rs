// Signing of the packages (tools/codesign.py of the JFLib repository, docs/PACKAGING.md
// "Signing"): Authenticode by signtool for the Windows programs, the setup and the MSIX; GPG in
// Linux (WSL on Windows) for the AppImages, the apt repository and the Flatpak.  What cannot be
// signed is said and left unsigned; with --sign every gap is an error.
//
// Configuration (environment; nothing of it belongs in a repository):
//   jfgui_SIGN_PFX + jfgui_SIGN_PFX_PASS   a code-signing certificate with its key, or
//   jfgui_SIGN_THUMBPRINT                  one in the user's certificate store (SHA-1 thumbprint)
//   jfgui_SIGN_TIMESTAMP                   an RFC 3161 time-stamp server (recommended)
//   jfgui_SIGNTOOL                         signtool.exe (default: PATH, the newest Windows Kit)
//   jfgui_GPG_KEY                          the GPG key (id or fingerprint)
//   jfgui_GPG_HOME, jfgui_GPG_PASSPHRASE   its GnuPG home in Linux, its passphrase

use std::cell::RefCell;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::linux::Linux;
use crate::package;

struct Signtool {
    tool: PathBuf,
    args: Vec<String>,
    what: String,
}

pub(crate) struct Signing {
    required: bool,
    pe: Option<Signtool>,
    /// the GPG key and the Linux it is used in
    gpg: Option<(String, Linux)>,
    work: PathBuf,
    pub done: RefCell<Vec<String>>,
    pub skipped: RefCell<Vec<String>>,
}

fn signtool() -> Option<PathBuf> {
    if let Some(p) = env::var_os("jfgui_SIGNTOOL") {
        return Some(PathBuf::from(p)).filter(|p| p.is_file());
    }
    if let Some(p) = env::split_paths(&env::var_os("PATH").unwrap_or_default()).map(|d| d.join("signtool.exe")).find(|p| p.is_file()) {
        return Some(p);
    }
    let kits = PathBuf::from(env::var_os("ProgramFiles(x86)").unwrap_or_else(|| "C:\\Program Files (x86)".into())).join("Windows Kits/10/bin");
    let key = |p: &PathBuf| p.file_name().and_then(|n| n.to_str()).map(|n| n.split('.').map(|v| v.parse::<u32>().unwrap_or(0)).collect::<Vec<_>>());
    fs::read_dir(kits).ok()?.flatten().map(|e| e.path()).filter(|p| p.join("x64/signtool.exe").is_file()).max_by_key(key)
        .map(|p| p.join("x64/signtool.exe"))
}

fn var(name: &str) -> Option<String> {
    env::var(name).ok().filter(|v| !v.is_empty())
}

/// the certificate table of a signed PE file: (offset, size), None when unsigned
pub(crate) fn pe_signature(b: &[u8]) -> Option<(usize, usize)> {
    let u16_at = |at: usize| b.get(at..at + 2).map(|x| u16::from_le_bytes([x[0], x[1]]) as usize);
    let u32_at = |at: usize| b.get(at..at + 4).map(|x| u32::from_le_bytes([x[0], x[1], x[2], x[3]]) as usize);
    if !b.starts_with(b"MZ") {
        return None;
    }
    let pe = u32_at(0x3C)?;
    if b.get(pe..pe + 4)? != b"PE\0\0" {
        return None;
    }
    let opt = pe + 24;
    let dirs = opt + match u16_at(opt)? { 0x10B => 96, 0x20B => 112, _ => return None };
    if u32_at(dirs - 4)? < 5 {
        return None;
    }
    let (off, size) = (u32_at(dirs + 32)?, u32_at(dirs + 36)?);
    (off > 0 && size > 0 && off + size == b.len()).then_some((off, size))
}

impl Signing {
    /// what can be signed here: Authenticode when Windows files are made, GPG when Linux ones are
    pub(crate) fn new(required: bool, windows: bool, linux: bool, work: &Path) -> Result<Signing, String> {
        let mut s = Signing { required, pe: None, gpg: None, work: work.to_path_buf(), done: RefCell::new(Vec::new()), skipped: RefCell::new(Vec::new()) };
        if windows {
            match (cfg!(windows), signtool(), var("jfgui_SIGN_PFX"), var("jfgui_SIGN_THUMBPRINT")) {
                (false, ..) => s.skip("Authenticode: only on Windows (signtool)")?,
                (_, None, ..) => s.skip("Authenticode: signtool.exe not found (the Windows SDK, or jfgui_SIGNTOOL)")?,
                (_, Some(tool), Some(pfx), _) => {
                    if !Path::new(&pfx).is_file() {
                        s.skip(&format!("Authenticode: jfgui_SIGN_PFX names no file ({pfx})"))?;
                    } else {
                        let mut args = vec!["/f".to_string(), pfx.clone()];
                        if let Some(pw) = var("jfgui_SIGN_PFX_PASS") {
                            args.extend(["/p".to_string(), pw]);
                        }
                        let name = Path::new(&pfx).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                        s.pe = Some(Signtool { tool, args, what: format!("certificate {name}") });
                    }
                }
                (_, Some(tool), None, Some(thumb)) => {
                    let t = thumb.replace(' ', "");
                    s.pe = Some(Signtool { tool, what: format!("certificate {}... from the store", &t[..t.len().min(8)]), args: vec!["/sha1".into(), t] });
                }
                _ => s.skip("Authenticode: no certificate (jfgui_SIGN_PFX with jfgui_SIGN_PFX_PASS, or jfgui_SIGN_THUMBPRINT)")?,
            }
        }
        if linux {
            match var("jfgui_GPG_KEY") {
                None => s.skip("GPG: no key (jfgui_GPG_KEY): AppImages, apt repository and Flatpak unsigned")?,
                Some(key) => match Linux::with("gpg") {
                    Ok(l) => s.gpg = Some((key, l)),
                    Err(why) => s.skip(&format!("GPG: {why}"))?,
                },
            }
        }
        Ok(s)
    }

    /// a signature that cannot be made: an error with --sign, else said
    pub(crate) fn skip(&self, why: &str) -> Result<(), String> {
        if self.required {
            return Err(format!("--sign: {why}"));
        }
        println!("  [sign] not signed: {why}");
        self.skipped.borrow_mut().push(why.to_string());
        Ok(())
    }

    pub(crate) fn has_gpg(&self) -> bool {
        self.gpg.is_some()
    }

    fn signtool_run(&self, paths: &[PathBuf], desc: Option<&str>) -> Result<(), String> {
        let Some(st) = &self.pe else { return Ok(()) };
        let mut cmd = Command::new(&st.tool);
        cmd.args(["sign", "/fd", "SHA256"]).args(&st.args);
        if let Some(d) = desc {
            cmd.args(["/d", d]);
        }
        if let Some(ts) = var("jfgui_SIGN_TIMESTAMP") {
            cmd.args(["/tr", &ts, "/td", "SHA256"]);
        }
        let out = cmd.args(paths).stdin(Stdio::null()).output().map_err(|e| format!("{}: {e}", st.tool.display()))?;
        if !out.status.success() {
            let text = String::from_utf8_lossy(&out.stdout).to_string() + &String::from_utf8_lossy(&out.stderr);
            let text = match var("jfgui_SIGN_PFX_PASS") { Some(pw) => text.replace(&pw, "***"), None => text };
            return Err(format!("signtool failed:\n{}", text.trim()));
        }
        Ok(())
    }

    /// Authenticode for these programs (and the setup)
    pub(crate) fn windows(&self, paths: &[PathBuf], desc: Option<&str>) -> Result<(), String> {
        let Some(st) = &self.pe else { return Ok(()) };
        if paths.is_empty() {
            return Ok(());
        }
        self.signtool_run(paths, desc)?;
        for p in paths {
            if pe_signature(&package::read(p)?).is_none() {
                return Err(format!("signtool reported success, but {} has no signature", p.display()));
            }
            self.done.borrow_mut().push(format!("Authenticode ({}): {}", st.what, p.file_name().map(|n| n.to_string_lossy()).unwrap_or_default()));
        }
        Ok(())
    }

    /// an MSIX package (signtool writes AppxSignature.p7x; the certificate's subject must be the
    /// manifest's Publisher)
    pub(crate) fn msix(&self, path: &Path, publisher: &str) -> Result<(), String> {
        let Some(st) = &self.pe else {
            return self.skip(&format!("MSIX {}: no certificate (Add-AppxPackage needs a trusted signature)",
                                      path.file_name().map(|n| n.to_string_lossy()).unwrap_or_default()));
        };
        if let Err(e) = self.signtool_run(&[path.to_path_buf()], None) {
            if e.to_lowercase().contains("0x8007000b") {
                return Err(format!("{e}\nMSIX: the certificate's subject is not the package's Publisher ({publisher}; PACKAGE msix_publisher= or jfgui_MSIX_PUBLISHER)"));
            }
            return Err(e);
        }
        if !crate::zip::names(path)?.iter().any(|n| n == "AppxSignature.p7x") {
            return Err(format!("signtool reported success, but {} has no AppxSignature.p7x", path.display()));
        }
        self.done.borrow_mut().push(format!("MSIX ({}): {}", st.what, path.file_name().map(|n| n.to_string_lossy()).unwrap_or_default()));
        Ok(())
    }

    /// the GPG settings handed to Linux
    fn gpg_env(&self) -> Vec<(&'static str, String)> {
        ["jfgui_GPG_HOME", "jfgui_GPG_PASSPHRASE"].into_iter().filter_map(|n| var(n).map(|v| (n, v))).collect()
    }

    /// gpg's command line in a script: GPG ("gpg --batch --yes [--homedir]") and SIGN (with the passphrase)
    const GPG_PRELUDE: &'static str = "set -e\nGPG=\"gpg --batch --yes\"\n[ -n \"$jfgui_GPG_HOME\" ] && GPG=\"$GPG --homedir $jfgui_GPG_HOME\"\n\
        sign() { if [ -n \"$jfgui_GPG_PASSPHRASE\" ]; then printf %s \"$jfgui_GPG_PASSPHRASE\" | $GPG --pinentry-mode loopback --passphrase-fd 0 \"$@\"; \
        else $GPG \"$@\"; fi; }\n";

    /// an armored detached signature of `data` and the key's armored public part
    pub(crate) fn gpg_detach(&self, data: &[u8]) -> Result<(String, String), String> {
        let Some((key, linux)) = &self.gpg else { return Err("no GPG key".to_string()) };
        let d = self.work.join("gpg");
        let (input, sig, public, script) = (d.join("data"), d.join("data.asc"), d.join("key.asc"), d.join("sign.sh"));
        package::write(&input, data)?;
        let _ = fs::remove_file(&sig);
        package::write(&script, format!("{}sign --local-user \"$1\" --armor --detach-sign --output \"$3\" \"$2\"\n\
            $GPG --armor --export \"$1\" > \"$4\"\n[ -s \"$4\" ] || {{ echo \"gpg has no public key $1\" >&2; exit 1; }}\n", Self::GPG_PRELUDE).as_bytes())?;
        let out = linux.run_env(&script, &[key.clone(), linux.path(&input), linux.path(&sig), linux.path(&public)], &self.gpg_env())?;
        if !out.status.success() {
            return Err(format!("gpg ({}):\n{}", linux.name(), String::from_utf8_lossy(&out.stderr).trim()));
        }
        let text = |p: &Path| fs::read_to_string(p).map_err(|e| format!("{}: {e}", p.display()));
        Ok((text(&sig)?, text(&public)?))
    }

    /// InRelease, Release.gpg and the public key repo.gpg of the apt repository in `dir`
    pub(crate) fn apt_release(&self, dir: &Path) -> Result<(), String> {
        let Some((key, linux)) = &self.gpg else { return Ok(()) };
        let script = self.work.join("gpg").join("release.sh");
        package::write(&script, format!("{}cd \"$2\"\nsign --local-user \"$1\" --digest-algo SHA512 --clearsign -o InRelease Release\n\
            sign --local-user \"$1\" --digest-algo SHA512 --armor --detach-sign -o Release.gpg Release\n$GPG --export \"$1\" > repo.gpg\n\
            [ -s repo.gpg ] || {{ echo \"gpg has no public key $1\" >&2; exit 1; }}\n", Self::GPG_PRELUDE).as_bytes())?;
        let out = linux.run_env(&script, &[key.clone(), linux.path(dir)], &self.gpg_env())?;
        if !out.status.success() {
            return Err(format!("gpg ({}):\n{}", linux.name(), String::from_utf8_lossy(&out.stderr).trim()));
        }
        self.done.borrow_mut().push(format!("apt repository {} (InRelease, Release.gpg, repo.gpg)", dir.display()));
        Ok(())
    }

    /// the key and the GPG settings for a script that signs itself (Flatpak)
    pub(crate) fn gpg_for_script(&self) -> Option<(String, Vec<(&'static str, String)>)> {
        self.gpg.as_ref().map(|(k, _)| (k.clone(), self.gpg_env()))
    }

    pub(crate) fn report(&self) {
        let done = self.done.borrow();
        println!("signed: {}", if done.is_empty() { "nothing".to_string() } else { done.join(", ") });
        for why in self.skipped.borrow().iter() {
            println!("not signed: {why}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_signature_in_a_bare_file() {
        assert!(pe_signature(b"MZ").is_none());
        assert!(pe_signature(&[0u8; 512]).is_none());
    }
}
