//! Shell-free Windows/.NET and Linux/Wine transport, shared by effects and codecs.
use alloc::{format, string::String, vec::Vec};
use core::sync::atomic::{AtomicU32, Ordering};

fn option(flag: &str, env: &str) -> Option<String> {
    plat::arg_value(flag)
        .or_else(|| plat::env(env))
        .filter(|s| !s.is_empty())
}
pub(crate) fn configured() -> bool {
    cfg!(windows) || option("--pdn-wine", "PAINTCRAFT_PDN_WINE").is_some()
}
pub(crate) fn helper_path() -> String {
    let exe = plat::current_exe().unwrap_or_default();
    let dir = exe.rsplit_once(['/', '\\']).map_or(".", |v| v.0);
    option("--pdn-host", "PAINTCRAFT_PDN_HOST")
        .unwrap_or_else(|| format!("{dir}/pdn-host/PaintCraft.PdnHost.dll"))
}
#[cfg(target_os = "linux")]
fn prefix() -> String {
    option("--pdn-wine-prefix", "WINEPREFIX")
        .unwrap_or_else(|| format!("{}/.wine", plat::env("HOME").unwrap_or_default()))
}
pub(crate) fn catalog_path(path: &str) -> Result<String, String> {
    #[cfg(windows)]
    {
        Ok(String::from(path))
    }
    #[cfg(target_os = "linux")]
    {
        super::paths::to_native(path, &prefix()).map_err(String::from)
    }
}

struct Invocation {
    program: String,
    args: Vec<String>,
    env: Vec<(String, String)>,
}
impl Invocation {
    fn new(helper: &str, args: &[&str], heartbeat: &str) -> Result<Self, String> {
        let mut env = alloc::vec![(String::from("DOTNET_ReadyToRun"), String::from("0"))];
        #[cfg(windows)]
        {
            let apphost = helper.to_ascii_lowercase().ends_with(".exe");
            let mut full = if apphost {
                Vec::new()
            } else {
                alloc::vec![String::from(helper)]
            };
            full.extend(args.iter().map(|s| String::from(*s)));
            full.extend([String::from("--watchdog"), String::from(heartbeat)]);
            if let Some(install) = option("--pdn-install", "PAINTDOTNET_DIR") {
                env.push((String::from("PAINTDOTNET_DIR"), install));
            }
            Ok(Self {
                program: if apphost {
                    String::from(helper)
                } else {
                    option("--pdn-dotnet", "PAINTCRAFT_PDN_DOTNET")
                        .unwrap_or_else(|| String::from("dotnet"))
                },
                args: full,
                env,
            })
        }
        #[cfg(target_os = "linux")]
        {
            let program = option("--pdn-wine", "PAINTCRAFT_PDN_WINE")
                .ok_or_else(|| String::from("Linux-DLL-Brücke: --pdn-wine und eine Windows-.NET-/Paint.NET-Installation konfigurieren."))?;
            let cwd = plat::cwd();
            let map = |s: &str| super::paths::to_windows(s, &cwd).map_err(String::from);
            let dotnet = option("--pdn-dotnet", "PAINTCRAFT_PDN_DOTNET")
                .unwrap_or_else(|| String::from("C:/Program Files/dotnet/dotnet.exe"));
            let mut full = if helper.to_ascii_lowercase().ends_with(".exe") {
                alloc::vec![map(helper)?]
            } else {
                alloc::vec![map(&dotnet)?, map(helper)?]
            };
            for (i, arg) in args.iter().enumerate() {
                let is_path =
                    (i == 0 && !arg.starts_with("--")) || (i > 0 && args[i - 1] == "--root");
                full.push(if is_path {
                    map(arg)?
                } else {
                    String::from(*arg)
                });
            }
            full.extend([String::from("--watchdog"), map(heartbeat)?]);
            let prefix = prefix();
            if !prefix.starts_with('/') {
                return Err(String::from(
                    "Das Wine-Präfix muss ein absoluter Linux-Pfad sein.",
                ));
            }
            env.push((String::from("WINEPREFIX"), prefix));
            env.push((String::from("WINEDEBUG"), String::from("-all")));
            if let Some(install) = option("--pdn-install", "PAINTDOTNET_DIR") {
                env.push((String::from("PAINTDOTNET_DIR"), map(&install)?));
            }
            Ok(Self {
                program,
                args: full,
                env,
            })
        }
    }
}

// Wine can launch descendants through its server outside the Unix process group.
// A host-side watchdog also stops those workers if the editor exits or cancels.
struct Heartbeat {
    path: String,
    file: Option<plat::File>,
    last: u64,
}
static SEQUENCE: AtomicU32 = AtomicU32::new(0);
impl Heartbeat {
    fn new() -> Result<Self, String> {
        let dir =
            plat::temp_dir().ok_or_else(|| String::from("Temporärverzeichnis nicht verfügbar."))?;
        for _ in 0..16 {
            let path = format!(
                "{dir}/paintcraft-pdn-{:x}-{:x}.watch",
                plat::time_us(),
                SEQUENCE.fetch_add(1, Ordering::Relaxed)
            );
            if let Some(file) = plat::File::create_new(&path) {
                let mut result = Self {
                    path,
                    file: Some(file),
                    last: 0,
                };
                result.tick(true)?;
                return Ok(result);
            }
        }
        Err(String::from(
            "Plugin-Überwachung konnte nicht angelegt werden.",
        ))
    }
    fn tick(&mut self, force: bool) -> Result<(), String> {
        let now = plat::time_us();
        if force || now.saturating_sub(self.last) >= 200_000 {
            let data = now.to_le_bytes();
            if self.file.as_ref().and_then(|f| f.write_at(0, &data)) != Some(data.len()) {
                return Err(String::from(
                    "Plugin-Überwachung konnte nicht aktualisiert werden.",
                ));
            }
            self.last = now;
        }
        Ok(())
    }
}
impl Drop for Heartbeat {
    fn drop(&mut self) {
        self.file = None;
        plat::delete(&self.path);
    }
}

pub(crate) fn exchange(
    helper: &str,
    args: &[&str],
    input: &[u8],
    limit: usize,
    timeout_ms: u64,
    cancel: impl Fn() -> bool,
) -> Result<Vec<u8>, String> {
    let mut heartbeat = Heartbeat::new()?;
    let launch = Invocation::new(helper, args, &heartbeat.path)?;
    let args: Vec<_> = launch.args.iter().map(String::as_str).collect();
    let env: Vec<_> = launch
        .env
        .iter()
        .map(|(k, v)| (k.as_str(), Some(v.as_str())))
        .collect();
    let mut child = plat::process::Command {
        program: &launch.program,
        args: &args,
        cwd: None,
        env: &env,
    }
    .spawn()
    .map_err(|e| {
        format!("Plugin-Host konnte nicht gestartet werden ({e}). .NET-/Wine-Konfiguration prüfen.")
    })?;
    let start = plat::time_us();
    let (mut sent, mut closed) = (0, false);
    let mut output = Vec::new();
    let mut errors = Vec::new();
    let mut buffer = [0; 16384];
    loop {
        if cancel() || timeout_ms != 0 && plat::time_us().saturating_sub(start) / 1000 > timeout_ms
        {
            return Err(String::from(
                "Plugin abgebrochen oder Zeitlimit überschritten.",
            ));
        }
        heartbeat.tick(false)?;
        let exit = child
            .try_wait()
            .map_err(|_| String::from("Plugin-Prozess nicht erreichbar."))?;
        let mut pending_output = false;
        // Drain both streams while sending, so Wine diagnostics cannot fill a pipe
        // and deadlock a large image upload before the helper starts reading it.
        for stderr in [false, true] {
            let mut drained = false;
            for _ in 0..32 {
                let read = if stderr {
                    child.read_stderr(&mut buffer)
                } else {
                    child.read_stdout(&mut buffer)
                };
                match read {
                    Ok(Some(0)) | Ok(None) => {
                        drained = true;
                        break;
                    }
                    Ok(Some(n)) if stderr => {
                        let n = n.min(8192usize.saturating_sub(errors.len()));
                        errors.extend_from_slice(&buffer[..n]);
                    }
                    Ok(Some(n)) => {
                        if n > limit.saturating_sub(output.len()) {
                            return Err(String::from("Ungültige Größe der Plugin-Antwort."));
                        }
                        output.extend_from_slice(&buffer[..n]);
                    }
                    Err(e) => {
                        return Err(format!("Plugin-Ausgabe konnte nicht gelesen werden ({e})."));
                    }
                }
            }
            pending_output |= !drained;
        }
        if let Some(code) = exit {
            if pending_output {
                continue;
            }
            if code == 2 {
                return Ok(Vec::new());
            }
            if code != 0 {
                return Err(format!(
                    "Plugin-Host beendet ({code}): {}",
                    String::from_utf8_lossy(&errors)
                ));
            }
            if sent < input.len() {
                return Err(String::from("Plugin-Host hat die Eingabe abgebrochen."));
            }
            return Ok(output);
        }
        if sent < input.len() {
            match child.write_stdin(&input[sent..]) {
                Ok(Some(0)) => {
                    return Err(String::from("Plugin-Host hat die Eingabe abgebrochen."));
                }
                Ok(Some(n)) => sent += n,
                Ok(None) => {}
                Err(e) => {
                    return Err(format!(
                        "Plugin-Eingabe abgebrochen ({e}): {}",
                        String::from_utf8_lossy(&errors)
                    ));
                }
            }
        }
        if sent == input.len() && !closed {
            child.close_stdin();
            closed = true;
        }
        plat::sleep_ms(1);
    }
}
