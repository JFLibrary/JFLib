// watch -- the core of the observation mode (`modeltool --watch FILE`): the script and every module it asked for,
// watched by their content (an editor may keep the time stamp or replace the file), built again when one changes;
// what a build gives for the window (watch_ui.rs): the model, its summary and findings, or the errors with their
// places, and the host's warnings with theirs.  No window here, so the console test (selftest group "watch") runs
// it with real files.

use alloc::format;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;

use crate::host::Model;
use crate::run;

/// what the last build of a watched script gave
pub struct State {
    /// the model of the last build that ran (kept while the script has an error)
    pub model: Option<Rc<Model>>,
    /// the model's name or the file's
    pub title: String,
    /// every check passed (None: the script failed)
    pub ok: Option<bool>,
    /// "28 Teile, 444 Dreiecke, 0.8 x 0.8 x 0.8 m"
    pub summary: String,
    /// what did not pass, one sentence each
    pub findings: Vec<String>,
    /// the errors of a failed run: per problem its lines (file:line:col: message, the source line, the calls)
    pub errors: Vec<Vec<String>>,
    /// the host's warnings of the last build (it ran or failed): per warning its lines (file:line:col: Warnung:
    /// message, the source line), at most run::WARNINGS_SHOWN, then a line with how many more
    pub warnings: Vec<Vec<String>>,
    /// how many warnings the last build gave (also those not shown and not kept)
    pub warning_count: u64,
    /// what the script printed
    pub output: String,
    /// the number of builds so far
    pub builds: u32,
    /// how long the last build took (script, geometry, checks), ms
    pub ms: f64,
}

/// a script being watched
pub struct Watched {
    pub file: String,
    dir: String,
    params: Vec<(String, String)>,
    /// (path, content or None when it could not be read) of the script and of each module it asked for at the
    /// last build
    seen: Vec<(String, Option<Vec<u8>>)>,
    pub state: State,
}

/// "0.8" with at most three decimals
fn short(v: f64) -> String {
    geom::fnum(geom::math::floor(v * 1000.0 + 0.5) / 1000.0)
}

impl Watched {
    pub fn new(file: &str, params: &[(String, String)]) -> Watched {
        let file = file.replace('\\', "/");
        Watched {
            dir: run::script_dir(&file),
            file,
            params: params.to_vec(),
            seen: Vec::new(),
            state: State { model: None, title: String::new(), ok: None, summary: String::new(), findings: Vec::new(), errors: Vec::new(), warnings: Vec::new(), warning_count: 0, output: String::new(), builds: 0, ms: 0.0 },
        }
    }

    /// whether the script or one of its modules changed since the last build (or there was none)
    pub fn changed(&self) -> bool {
        self.seen.is_empty() || self.seen.iter().any(|(p, c)| plat::read_file(p) != *c)
    }

    /// the files watched now
    pub fn files(&self) -> Vec<String> {
        self.seen.iter().map(|(p, _)| p.clone()).collect()
    }

    /// Build the script again: the state gets the new model, or the errors (the old model stays).
    pub fn build(&mut self) {
        let t0 = plat::time_us();
        let name = String::from(self.file.rsplit('/').next().unwrap_or(&self.file));
        self.state.builds += 1;
        let Some(bytes) = plat::read_file(&self.file) else {
            self.seen = alloc::vec![(self.file.clone(), None)];
            self.state.ok = None;
            self.state.errors = alloc::vec![alloc::vec![format!("{}: Fehler: die Datei lässt sich nicht lesen", self.file)]];
            self.state.warnings.clear();
            self.state.warning_count = 0;
            self.state.ms = 0.0;
            return;
        };
        let src = String::from_utf8_lossy(&bytes).into_owned();
        let mut asked: Vec<(String, Option<Vec<u8>>)> = Vec::new();
        let dir = self.dir.clone();
        let mut loader = |p: &str| -> Result<String, String> {
            let r = run::file_loader(&dir, p);
            asked.push((String::from(p), plat::read_file(p)));
            r
        };
        let r = run::run_script(&self.file, &src, &self.params, &mut loader);
        self.seen = alloc::vec![(self.file.clone(), Some(bytes))];
        self.seen.extend(asked);
        self.state.output = r.output.clone();
        let mut sources = alloc::vec![(r.file.clone(), src.clone())];
        sources.extend(r.imports.iter().cloned());
        self.state.warnings = run::format_warnings(&r, &sources).iter().map(|w| w.lines().map(String::from).collect()).collect();
        self.state.warning_count = r.warnings.len() as u64 + r.warnings_dropped;
        if !r.ok() {
            self.state.ok = None;
            self.state.errors = r.problems.iter().map(|p| run::format_problem(p, &sources).lines().map(String::from).collect()).collect();
            self.state.ms = (plat::time_us().saturating_sub(t0)) as f64 / 1000.0;
            return;
        }
        let a = run::analyse(&r.model);
        let s = a.whole.size();
        self.state.title = if r.model.name.is_empty() { name } else { r.model.name.clone() };
        self.state.ok = Some(a.ok());
        self.state.summary = format!("{} Teile, {} Dreiecke, {} x {} x {} m", r.model.parts.len(), a.whole.triangles, short(s.x), short(s.y), short(s.z));
        self.state.findings = run::findings(&r.model, &a);
        self.state.errors.clear();
        self.state.model = Some(Rc::new(r.model));
        self.state.ms = (plat::time_us().saturating_sub(t0)) as f64 / 1000.0;
    }
}
