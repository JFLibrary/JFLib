// jfWord native/runtime lifecycle adapter.
use crate::*;
impl App for Word {
    fn actions(&self) -> &'static [agent_core::ActionSpec] { super::agent::actions() }
    fn action_revision(&self) -> Option<u64> { Some(self.document_revision()) }
    fn action_available(&self, spec: &agent_core::ActionSpec) -> Result<(), agent_core::ActionError> { self.domain_available(spec) }
    fn action(&mut self, ui: &mut Ui, spec: &agent_core::ActionSpec, input: &agent_core::Value,
        context: &agent_core::ActionContext) -> Result<agent_core::ActionResult, agent_core::ActionError> {
        self.domain_action(ui, spec, input, context)
    }
    fn action_inspect(&self, query: &agent_core::Value) -> Result<agent_core::Value, agent_core::ActionError> { self.document_inspect(query) }
    fn action_resource_info(&self, uri: &str) -> Result<agent_core::Resource, agent_core::ActionError> { self.document_resource_info(uri) }
    fn action_resource_read(&self, uri: &str, offset: u64, limit: usize) -> Result<Vec<u8>, agent_core::ActionError> { self.document_resource_read(uri, offset, limit) }
    fn init(&mut self, ui: &mut Ui) -> bool {
        let args = base::Args(plat::args());
        // Headless checks never touch the real profile unless a directory is explicit.
        let library_path = match args.value("--library-dir") {
            Some(dir) if !dir.is_empty() => Some(format!("{dir}/sources.jflib")),
            _ if !ui.headless() => {
                plat::config_dir().map(|dir| format!("{dir}/jfWord/sources.jflib"))
            }
            _ => None,
        };
        self.library.configure(library_path);
        let settings_path = match args.value("--settings-dir") {
            Some(dir) if !dir.is_empty() => Some(format!("{dir}/recent.ini")),
            _ if !ui.headless() => plat::config_dir().map(|dir| format!("{dir}/jfWord/recent.ini")),
            _ => None,
        };
        self.shell.configure(settings_path);
        self.preview_mode = args.flag("--preview");
        self.structured_edit = args.flag("--structure");
        self.shell.home = !args.flag("--editor") && !self.preview_mode;
        self.shell.has_document = !self.shell.home;
        self.show_sources = args.flag("--sources") && !args.flag("--no-sources");
        self.show_navigation = args.flag("--navigation");
        if let Some(path) = base::Args(plat::args()).value("--file") {
            if let Err(e) = self.open_document(ui, path) {
                self.message = format!("Öffnen fehlgeschlagen: {e:?}");
            }
        }
        if !self.shell.home && self.path.is_empty() && !self.preview_mode {
            self.reset_paper_view = true;
            let field = self.text_id(self.selected_id(), 0);
            ui.focus(field);
        }
        true
    }
    fn frame(&mut self, ui: &mut Ui) {
        #[cfg(target_arch = "wasm32")]
        self.poll_browser_save(ui);
        self.frame_ui(ui);
    }
    fn command(&mut self, ui: &mut Ui, id: u32, arg: u32) -> bool {
        if id == UIC_DROP {
            if let Some(path) = ui.drop_path(0).map(String::from) {
                self.shell.open_target = path;
                return self.handle_command(ui, OPEN_RECENT, 0);
            }
        }
        self.handle_command(ui, id, arg)
    }
}
