// Optional system integrations run off the UI thread; only complete results are imported.
use crate::app::Pn;
use crate::app::constants::*;
use alloc::{boxed::Box, format, string::String, sync::Arc, vec::Vec};
use core::sync::atomic::{AtomicBool, Ordering};
use rt::sync::SpinLock;
use ui::*;

pub(crate) static BUSY: AtomicBool = AtomicBool::new(false);
struct Job {
    cancel: AtomicBool,
    result: SpinLock<Option<Result<Vec<u8>, String>>>,
}
#[derive(Default)]
pub(crate) struct Services {
    job: Option<Arc<Job>>,
    worker: Option<plat::Thread>,
    scan: bool,
    auto_checked: bool,
    pub(crate) update_status: String,
    open: Shared<i32>,
}
impl Drop for Services {
    fn drop(&mut self) {
        if let Some(j) = &self.job {
            j.cancel.store(true, Ordering::Release);
        }
        if let Some(t) = self.worker.take() {
            t.join();
        }
    }
}
pub(crate) fn states(p: &mut Pn, ui: &mut Ui) {
    crate::app::commands::pn_cmd_set(
        ui,
        CMD_ACQUIRE,
        if cfg!(target_arch = "wasm32") || p.services.job.is_some() {
            CF_DISABLED
        } else {
            0
        },
    );
}
pub(crate) fn command(p: &mut Pn, ui: &mut Ui, id: u32, _arg: u32) -> bool {
    if id != CMD_ACQUIRE {
        return false;
    }
    start(p, ui, true);
    true
}
pub(crate) fn check_updates(p: &mut Pn, ui: &mut Ui) {
    start(p, ui, false);
}
fn start(p: &mut Pn, ui: &mut Ui, scan: bool) {
    if p.services.job.is_some() {
        return;
    }
    let fixture = plat::arg_value(if scan { "--acquire-file" } else { "--update-feed" });
    let beta = p.st.st.update_beta != 0;
    let job = Arc::new(Job {
        cancel: AtomicBool::new(false),
        result: SpinLock::new(None),
    });
    let worker_job = job.clone();
    let worker = plat::thread_spawn(
        0,
        Box::new(move || {
            let result = run(scan, beta, fixture.as_deref(), &worker_job);
            *worker_job.result.lock() = Some(result);
            plat::wake();
        }),
    );
    if worker.is_none() {
        crate::integrations::file_io::fio_error(
            p,
            Some("Systemdienst"),
            "Der Hintergrund-Thread konnte nicht gestartet werden.",
        );
        return;
    }
    p.services.scan = scan;
    p.services.job = Some(job);
    p.services.worker = worker;
    BUSY.store(true, Ordering::Release);
    if !scan {
        p.services.update_status = String::from("Veröffentlichungen werden geprüft …");
    }
    ui.repass();
}
pub(crate) fn frame(p: &mut Pn, ui: &mut Ui) {
    if !p.services.auto_checked {
        p.services.auto_checked = true;
        if p.st.st.update_auto != 0 && !ui.headless() {
            check_updates(p, ui);
        }
    }
    if let Some(result) = p.services.job.as_ref().and_then(|j| j.result.lock().take()) {
        if let Some(t) = p.services.worker.take() {
            t.join();
        }
        p.services.job = None;
        BUSY.store(false, Ordering::Release);
        match result {
            Ok(bytes) if p.services.scan && !bytes.is_empty() => {
                let error = crate::integrations::file_io::fio_acquired(p, ui, &bytes);
                if error != 0 {
                    crate::integrations::file_io::fio_error(
                        p,
                        Some("Bilderfassung"),
                        &format!("Das erfasste Bild ließ sich nicht öffnen ({error})."),
                    );
                }
            }
            Ok(bytes) if !p.services.scan => p.services.update_status = String::from_utf8_lossy(&bytes).into_owned(),
            Ok(_) => {}
            Err(e) if p.services.scan => crate::integrations::file_io::fio_error(p, Some("Bilderfassung"), &e),
            Err(e) => p.services.update_status = e,
        }
        ui.repass();
    }
    if p.services.job.is_some() {
        ui.timer(0x5111, 50);
    }
    let open = p.services.open.clone();
    open.set((p.services.scan && p.services.job.is_some()) as i32);
    if ui.window_begin(
        0x60E4,
        "Scanner oder Kamera",
        Some(&open),
        WF_DIALOG | WF_MODAL | WF_AUTOHEIGHT | WF_NOCLOSE,
        None,
    ) {
        ui.text(TS_BODY|TX_WRAP,crate::preferences::localization::tr("Das Gerät wird über WIA (Windows) oder SANE scanimage (Linux) angesprochen. Erst eine vollständige Aufnahme wird als neues Bild geöffnet."));
        if ui.button(0x90021, crate::preferences::localization::tr("Abbrechen")) & EV_ACT != 0 {
            if let Some(j) = &p.services.job {
                j.cancel.store(true, Ordering::Release);
            }
        }
    }
    ui.window_end();
}
#[cfg(target_arch = "wasm32")]
fn run(_scan: bool, _beta: bool, _fixture: Option<&str>, _job: &Job) -> Result<Vec<u8>, String> {
    Err(String::from("Systemdienste sind in der Desktop-Version verfügbar."))
}
#[cfg(not(target_arch = "wasm32"))]
fn run(scan: bool, beta: bool, fixture: Option<&str>, job: &Job) -> Result<Vec<u8>, String> {
    if scan {
        if let Some(path) = fixture {
            return plat::read_file(path).ok_or_else(|| String::from("Testaufnahme nicht lesbar."));
        }
    }
    let mut args: Vec<&str> = Vec::new();
    let program;
    if scan {
        if cfg!(windows) {
            program = "powershell.exe";
            args.extend_from_slice(&["-NoProfile","-NonInteractive","-Command", "try { $wia = New-Object -ComObject WIA.CommonDialog; $img = $wia.ShowAcquireImage(0,0,0,'{B96B3CAF-0728-11D3-9D7B-0000F81EF32E}',$false,$true,$false); if ($null -eq $img) { exit 2 }; $out = [Console]::OpenStandardOutput(); $bytes = [byte[]]$img.FileData.BinaryData; $out.Write($bytes,0,$bytes.Length) } catch { [Console]::Error.Write($_.Exception.Message); exit 1 }"]);
        } else {
            program = "scanimage";
            args.push("--format=png");
        }
    } else {
        program = if cfg!(windows) { "python" } else { "python3" };
        args.extend_from_slice(&[
            "-u",
            "-c",
            include_str!("../../update_check.py"),
            crate::preferences::PN_VERSION,
            if beta { "1" } else { "0" },
        ]);
        if let Some(path) = fixture {
            args.push(path);
        }
    }
    let mut child = (plat::process::Command {
        program,
        args: &args,
        cwd: None,
        env: &[],
    })
    .spawn()
    .map_err(|e| {
        format!(
            "{program} konnte nicht gestartet werden ({e}). Für Updates wird Python 3, für Linux-Scanner SANE benötigt."
        )
    })?;
    child.close_stdin();
    let mut out = Vec::new();
    let mut errors = Vec::new();
    let mut buf = [0u8; 16384];
    loop {
        if job.cancel.load(Ordering::Acquire) {
            return Ok(Vec::new());
        }
        for error in [false, true] {
            loop {
                let read = if error {
                    child.read_stderr(&mut buf)
                } else {
                    child.read_stdout(&mut buf)
                };
                match read {
                    Ok(Some(0)) | Ok(None) => break,
                    Ok(Some(n)) => {
                        let dst = if error { &mut errors } else { &mut out };
                        let cap = if error {
                            8192
                        } else if scan {
                            512 * 1024 * 1024
                        } else {
                            65536
                        };
                        if dst.len() + n > cap {
                            if error {
                                continue;
                            }
                            return Err(String::from("Die Antwort überschreitet die zulässige Größe."));
                        }
                        dst.extend_from_slice(&buf[..n]);
                    }
                    Err(e) => return Err(format!("Fehler beim Lesen der Antwort ({e}).")),
                }
            }
        }
        if let Some(code) = child.try_wait().map_err(|e| format!("Prozessfehler ({e})."))? {
            while let Ok(Some(n)) = child.read_stdout(&mut buf) {
                if n == 0 {
                    break;
                }
                if out.len() + n > 512 * 1024 * 1024 {
                    return Err(String::from("Antwort zu groß."));
                }
                out.extend_from_slice(&buf[..n]);
            }
            while let Ok(Some(n)) = child.read_stderr(&mut buf) {
                if n == 0 {
                    break;
                }
                let n = n.min(8192usize.saturating_sub(errors.len()));
                errors.extend_from_slice(&buf[..n]);
            }
            if code == 2 && scan && cfg!(windows) {
                return Ok(Vec::new());
            }
            if code != 0 {
                return Err(format!("{program} ({code}): {}", String::from_utf8_lossy(&errors)));
            }
            if !scan && out.is_empty() {
                return Err(format!(
                    "Updateprozess lieferte keine Antwort: {}",
                    String::from_utf8_lossy(&errors)
                ));
            }
            return Ok(out);
        }
        plat::sleep_ms(10);
    }
}
