// Asynchronous JSONL transport. Only this worker owns the helper process and pipe IO.
use alloc::{boxed::Box, collections::VecDeque, format, string::String, sync::Arc, vec::Vec};
use core::sync::atomic::{AtomicBool, Ordering};
use agent_core::{json, Value};
use rt::sync::SpinLock;

struct Channels {
    outgoing: SpinLock<VecDeque<Vec<u8>>>,
    incoming: SpinLock<VecDeque<Value>>,
    stop: AtomicBool,
}
pub(crate) struct Client { channels: Arc<Channels>, worker: Option<plat::Thread>, sequence: u64 }
fn push(ch: &Channels, value: Value) {
    ch.incoming.lock().push_back(value);
    plat::wake();
}
fn error(ch: &Channels, message: String) { push(ch, Value::object([("error", Value::string(message))])); }
impl Client {
    pub(crate) fn start(root: String, python: String, distro: String) -> Result<Self, String> {
        let channels = Arc::new(Channels { outgoing: SpinLock::new(VecDeque::new()), incoming: SpinLock::new(VecDeque::new()), stop: AtomicBool::new(false) });
        let ch = channels.clone();
        let worker = plat::thread_spawn(0, Box::new(move || {
            let script = format!("{root}/tools/build_manager.py");
            let program = if python.is_empty() { if cfg!(windows) { "python" } else { "python3" } } else { &python };
            let args = ["-u", &script, "--stdio", "--wsl-distro", &distro];
            let mut process = match (plat::process::Command { program, args: &args, cwd: Some(&root), env: &[("PYTHONIOENCODING", Some("utf-8"))] }).spawn() {
                Ok(p) => p,
                Err(e) => { error(&ch, format!("Python konnte nicht gestartet werden (Fehler {e}). Python-Pfad und Projektordner prüfen.")); return; }
            };
            let mut pending = Vec::new(); let mut sent = 0; let mut input = Vec::new();
            let mut bytes = [0u8; 16384];
            loop {
                if ch.stop.load(Ordering::Acquire) { break; }
                if sent == pending.len() { pending = ch.outgoing.lock().pop_front().unwrap_or_default(); sent = 0; }
                if sent < pending.len() {
                    match process.write_stdin(&pending[sent..]) {
                        Ok(Some(n)) => sent += n,
                        Ok(None) => {},
                        Err(e) => { error(&ch, format!("Verbindung unterbrochen (Schreiben: {e}).")); break; }
                    }
                }
                // Backpressure preserves status events and bounds memory even for large logs.
                if ch.incoming.lock().len() < 256 {
                    match process.read_stdout(&mut bytes) {
                        Ok(Some(0)) => break,
                        Ok(Some(n)) => {
                            input.extend_from_slice(&bytes[..n]);
                            while let Some(end) = input.iter().position(|b| *b == b'\n') {
                                let line: Vec<u8> = input.drain(..=end).collect();
                                match json::parse(&line) {
                                    Ok(value) => push(&ch, value),
                                    Err(_) => error(&ch, String::from("Ungültige Antwort des Build-Backends.")),
                                }
                            }
                            if input.len() > 1048576 { error(&ch, String::from("Backend-Antwort überschreitet 1 MiB.")); break; }
                        }
                        Ok(None) => {},
                        Err(e) => { error(&ch, format!("Verbindung unterbrochen (Lesen: {e}).")); break; }
                    }
                    if let Ok(Some(n)) = process.read_stderr(&mut bytes) {
                        if n > 0 { error(&ch, String::from_utf8_lossy(&bytes[..n]).into_owned()); }
                    }
                }
                plat::sleep_ms(10);
            }
            process.close_stdin();
            let end = plat::time_us() + 5_000_000;
            while plat::time_us() < end && matches!(process.try_wait(), Ok(None)) { plat::sleep_ms(20); }
            if !ch.stop.load(Ordering::Acquire) { error(&ch, String::from("Build-Backend beendet. Verbindung erneut herstellen.")); }
        })).ok_or_else(|| String::from("IO-Thread konnte nicht gestartet werden."))?;
        Ok(Self { channels, worker: Some(worker), sequence: 0 })
    }
    pub(crate) fn send(&mut self, method: &str, params: Value) {
        self.sequence += 1;
        let request = Value::object([("version", Value::u64(1)), ("id", Value::u64(self.sequence)),
            ("method", Value::string(method)), ("params", params)]);
        if let Ok(mut line) = json::stringify(&request) {
            line.push('\n'); self.channels.outgoing.lock().push_back(line.into_bytes());
        }
    }
    pub(crate) fn drain(&self) -> Vec<Value> { self.channels.incoming.lock().drain(..).collect() }
}
impl Drop for Client {
    fn drop(&mut self) {
        self.channels.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() { worker.join(); }
    }
}
