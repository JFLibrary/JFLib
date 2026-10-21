// App-level save checkpoints; transport, virtual files and durable requests belong to plat.
use crate::*;
pub(crate) struct PendingSave {
    pub(crate) id: u32,
    path: String,
    epoch: u64,
    revision: u64,
    preserved: bool,
    snapshot: Option<Vec<u8>>,
}
impl Word {
    pub(crate) fn start_browser_save(&mut self, ui: &mut Ui, path: String) -> bool {
        let Some(id) = plat::web::persist(&path) else {
            self.after_save = false;
            self.message = String::from("Browserspeicher ist nicht verfügbar. Die Datei ist nur temporär vorhanden.");
            return false;
        };
        let preserved = self.preserved();
        self.browser_save = Some(PendingSave {
            id, snapshot: if preserved { plat::read_file(&path) } else { None }, path,
            epoch: self.document_epoch, revision: self.editor.revision(), preserved,
        });
        self.message = String::from("Wird im Browserspeicher gesichert …");
        ui.invalidate(); true
    }
    pub(crate) fn poll_browser_save(&mut self, ui: &mut Ui) {
        use plat::web::RequestStatus as S;
        let Some(save) = self.browser_save.as_ref() else { return; };
        let state = plat::web::request_status(save.id).unwrap_or(S::Failed);
        if state == S::Pending { return; }
        let save = self.browser_save.take().unwrap();
        let unchanged = save.epoch == self.document_epoch && if save.preserved {
            self.document_file.as_ref().and_then(|f| f.encode(None).ok()) == save.snapshot
        } else { save.revision == self.editor.revision() };
        if state == S::Durable && unchanged {
            self.finish_save(ui, save.path, save.preserved);
            self.message = String::from("Gespeichert · Speicherung im Browser bestätigt");
        } else {
            self.after_save = false;
            self.message = String::from(match state {
                S::Durable => "Vorherige Fassung gespeichert · Neuere Änderungen noch ungespeichert",
                S::Quota => "Speichern fehlgeschlagen · Browserspeicher ist voll",
                S::Denied => "Speichern fehlgeschlagen · Zugriff auf Browserspeicher verweigert",
                S::Unsupported => "Dauerhafte Speicherung ist hier nicht verfügbar",
                S::Cancelled => "Speichern abgebrochen · Änderungen bleiben ungespeichert",
                _ => "Speichern fehlgeschlagen · Änderungen bleiben ungespeichert",
            });
        }
        ui.invalidate();
    }
}
