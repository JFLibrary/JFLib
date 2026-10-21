// Universal application automation through the existing input, command and render paths.
// Domain semantics remain in App::actions/action; no screen-coordinate inference is used.
use alloc::{format, string::String, vec::Vec};
use agent_core::{Access, ActionContext, ActionError, ActionResult, ActionSpec, Dispatcher, ErrorCode, Host, Resource, Value};
use agent_core::schema::{Property, Schema, OBJECT, STRING, U64};
use crate::{App, Ui, Runtime, Sem, consts::*};
use plat::policy::{AgentRule, Verdict};

#[derive(Default)]
pub(crate) struct State {
    enabled: bool,
    connected: bool,
    render_id: u64,
    images: Vec<(String, Vec<u8>)>,
    #[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
    server: Option<agent_io::ipc::Server>,
}

static NODE: Schema = Schema::Object { properties: &[
    Property { name: "id", schema: &U64, required: true },
    Property { name: "window", schema: &U64, required: false },
], additional: false };
static COMMAND: Schema = Schema::Object { properties: &[
    Property { name: "id", schema: &U64, required: true },
    Property { name: "arg", schema: &U64, required: false },
], additional: false };
static TEXT: Schema = Schema::Object { properties: &[
    Property { name: "text", schema: &STRING, required: true },
], additional: false };
static KEY: Schema = Schema::Object { properties: &[
    Property { name: "key", schema: &Schema::Unsigned { min: 1, max: 255 }, required: true },
    Property { name: "modifiers", schema: &Schema::Unsigned { min: 0, max: 7 }, required: false },
], additional: false };
static ACTIONS: [ActionSpec; 7] = [
    ActionSpec::read("ui.state", "Inspect window, focus, commands, capabilities and paginated widgets").schemas(&OBJECT, &OBJECT),
    ActionSpec::write("ui.command", "Execute an existing enabled numeric command").schemas(&COMMAND, &OBJECT),
    ActionSpec { revision_required: false, ..ActionSpec::write("ui.focus", "Focus an enabled widget by its semantic identifier").schemas(&NODE, &OBJECT) },
    ActionSpec::write("ui.click", "Activate an enabled widget through its real input handler").schemas(&NODE, &OBJECT),
    ActionSpec::write("ui.type", "Type Unicode text into the currently focused control").schemas(&TEXT, &OBJECT),
    ActionSpec::write("ui.key", "Deliver a virtual key and modifiers through the normal accelerator/input path").schemas(&KEY, &OBJECT),
    ActionSpec::read("ui.render", "Render a PNG and return a separately readable resource"),
];

struct UiHost<'a> {
    ui: &'a mut Ui,
    app: &'a mut dyn App,
    specs: Vec<ActionSpec>,
}
impl<'a> UiHost<'a> {
    fn new(ui: &'a mut Ui, app: &'a mut dyn App) -> Self {
        let mut specs = ACTIONS.to_vec();
        specs.extend_from_slice(app.actions());
        Self { ui, app, specs }
    }
    fn event(&mut self, event: plat::Event) {
        plat::evq_push(event);
        self.ui.process_events(self.app);
    }
    fn node(&self, input: &Value) -> Result<Sem, ActionError> {
        let id = number(input, "id", 0)?;
        let window = number(input, "window", 0)?;
        let node = self.ui.sems.iter().rev().find(|s| s.id == id && s.win as u32 == window)
            .ok_or_else(|| error(ErrorCode::NotFound, "Widget does not exist in the requested window"))?;
        if node.state & SS_DISABLED != 0 { return Err(error(ErrorCode::Unavailable, "Widget is disabled")); }
        if (node.layer as u32) < self.ui.modal_active() { return Err(error(ErrorCode::Unavailable, "A modal dialog blocks this widget")); }
        Ok(node.clone())
    }
    fn settle(&mut self) {
        self.ui.run_passes(self.app);
        self.ui.render();
    }
}
impl Host for UiHost<'_> {
    fn actions(&self) -> &[ActionSpec] { &self.specs }
    fn revision(&self) -> u64 { self.app.action_revision().unwrap_or(self.ui.frame as u64) }
    fn capabilities(&self) -> u64 { plat::capabilities().available }
    fn availability(&self, action: &ActionSpec) -> Result<(), ActionError> {
        if let Some(reason) = plat::capabilities().unavailable_reason(action.capabilities) {
            return Err(error(ErrorCode::CapabilityUnavailable, reason));
        }
        // The permission question is the user's: an agent cannot answer it through the GUI.
        if action.mutation && action.name.starts_with("ui.") && super::perm::asking() && !plat::policy::questions().is_empty() {
            return Err(error(ErrorCode::Unavailable, "A permission question waits for the user's answer"));
        }
        // Widget input can operate the active dialog; domain changes cannot bypass it.
        if action.mutation && self.ui.modal_active() != 0 && !matches!(action.name, "ui.focus" | "ui.click" | "ui.type" | "ui.key") {
            return Err(error(ErrorCode::Unavailable, "Close the active modal dialog before changing the application"));
        }
        if !action.name.starts_with("ui.") { self.app.action_available(action)?; }
        Ok(())
    }
    // the action's own access, tightened by the manifest (agent.deny / agent.confirm)
    fn access(&self, action: &ActionSpec) -> Access {
        match (action.access, plat::policy::agent_rule(action.name)) {
            (Access::Local, _) | (_, AgentRule::Deny) => Access::Local,
            (Access::Confirm, _) | (_, AgentRule::Confirm) => Access::Confirm,
            _ => Access::Open,
        }
    }
    // the user's answer (perm.rs shows the question): this run, stored, or still open
    fn confirm(&mut self, action: &ActionSpec, _input: &Value) -> Result<(), ActionError> {
        let details = |reason: &str| Value::object([("action", Value::string(action.name)), ("reason", Value::string(reason))]);
        match plat::policy::agent_ask(action.name, action.description) {
            Verdict::Allow => Ok(()),
            Verdict::Pending => {
                self.ui.invalidate(); // the question shows in the next frame
                Err(error(ErrorCode::ConfirmationRequired, "The user is asked to confirm this action; call it again after the answer").with_details(details("asked")))
            }
            Verdict::Deny => {
                let rule = format!("agent {}", action.name);
                let answered = plat::policy::decisions().iter().any(|d| d.rule == rule && d.answer.is_some());
                let text = if answered { "The user did not allow this action" } else { "Nobody can confirm this action in this run" };
                Err(error(ErrorCode::Forbidden, text).with_details(details(if answered { "user" } else { "unattended" })))
            }
        }
    }
    fn refused(&mut self, action: &ActionSpec) { plat::policy::agent_refused(action.name); }
    fn execute(&mut self, action: &ActionSpec, input: &Value, context: &ActionContext) -> Result<ActionResult, ActionError> {
        let value = match action.name {
            "ui.state" => return Ok(ActionResult::completed(self.inspect(input)?, self.revision())),
            "ui.command" => {
                let id = number(input, "id", 0)?;
                let arg = number(input, "arg", 0)?;
                if self.ui.cmd_flags(id) & CF_DISABLED != 0 { return Err(error(ErrorCode::Unavailable, "Command is disabled in the current state")); }
                let domain = self.app.actions().iter().find(|s| s.command_id == Some(id)).copied();
                if let Some(spec) = domain {
                    self.availability(&spec)?;
                    // a command of a domain action has that action's access
                    match self.access(&spec) {
                        Access::Open => {}
                        Access::Local => {
                            self.refused(&spec);
                            return Err(error(ErrorCode::Forbidden, "This action is not available to external control").with_details(Value::object([("action", Value::string(spec.name)), ("reason", Value::string("app"))])));
                        }
                        Access::Confirm => self.confirm(&spec, &Value::object([]))?,
                    }
                    let result = self.app.action(self.ui, &spec, &Value::object([]), context)?;
                    self.settle();
                    return Ok(ActionResult { revision: self.revision(), ..result });
                }
                self.ui.desc.cmds.iter().find(|c| c.id == id)
                    .ok_or_else(|| error(ErrorCode::NotFound, "Numeric command is not registered"))?;
                if !self.app.command(self.ui, id, arg) { return Err(error(ErrorCode::Unavailable, "Command is not available in the current state")); }
                Value::object([("handled", Value::Bool(true))])
            }
            "ui.focus" => {
                let node = self.node(input)?;
                if node.win != 0 { return Err(error(ErrorCode::Unsupported, "Use ui.click to focus a control in a secondary window")); }
                self.ui.focus(node.id);
                Value::object([("focused", Value::u64(node.id as u64))])
            }
            "ui.click" => {
                let node = self.node(input)?;
                if node.rect[2] <= 0 || node.rect[3] <= 0 { return Err(error(ErrorCode::Unavailable, "Widget has no visible hit rectangle")); }
                let event = plat::Event { win: node.win, x: node.rect[0] + node.rect[2] / 2, y: node.rect[1] + node.rect[3] / 2, btn: plat::BTN_LEFT, ..Default::default() };
                for typ in [plat::E_MOVE, plat::E_DOWN, plat::E_UP] { self.event(plat::Event { typ, ..event }); }
                Value::object([("activated", Value::u64(node.id as u64))])
            }
            "ui.type" => {
                if self.ui.focused() == 0 { return Err(error(ErrorCode::Unavailable, "No text control has keyboard focus")); }
                for ch in input.get("text").and_then(Value::as_str).unwrap_or("").chars() {
                    self.event(plat::Event { typ: plat::E_TEXT, data: ch as u32, ..Default::default() });
                }
                Value::object([("accepted", Value::Bool(true))])
            }
            "ui.key" => {
                let data = number(input, "key", 0)?;
                let mods = number(input, "modifiers", 0)? as u8;
                for typ in [plat::E_KEY_DOWN, plat::E_KEY_UP] { self.event(plat::Event { typ, data, mods, ..Default::default() }); }
                Value::object([("accepted", Value::Bool(true))])
            }
            "ui.render" => {
                self.settle();
                let image = self.ui.fb();
                let bytes = img_png::encode(&image.px, image.w as usize, image.h as usize, image.stride as usize, false);
                if bytes.len() > 16 * 1024 * 1024 { return Err(error(ErrorCode::LimitExceeded, "Rendered PNG exceeds 16 MiB")); }
                self.ui.agent.render_id += 1;
                let uri = format!("jfgui://render/{}.png", self.ui.agent.render_id);
                let resource = Resource { uri: uri.clone(), name: String::from("Application screenshot"), mime_type: String::from("image/png"), size: Some(bytes.len() as u64) };
                if self.ui.agent.images.len() == 4 { self.ui.agent.images.remove(0); }
                self.ui.agent.images.push((uri, bytes));
                let mut result = ActionResult::completed(Value::object([("image", resource.to_json())]), self.revision());
                result.resources.push(resource);
                return Ok(result);
            }
            _ => {
                let result = self.app.action(self.ui, action, input, context)?;
                self.settle();
                return Ok(ActionResult { revision: self.revision(), ..result });
            }
        };
        self.settle();
        Ok(ActionResult::completed(value, self.revision()))
    }
    fn inspect(&self, query: &Value) -> Result<Value, ActionError> {
        if query.get("domain").and_then(Value::as_str).is_some_and(|s| s != "ui") {
            return self.app.action_inspect(query);
        }
        let revision = self.revision();
        if query.get("sinceUiRevision").and_then(Value::as_u64) == Some(self.ui.frame as u64) {
            return Ok(Value::object([("revision", Value::u64(revision)), ("unchanged", Value::Bool(true))]));
        }
        let offset = query.get("offset").and_then(Value::as_u64).unwrap_or(0) as usize;
        let limit = query.get("limit").and_then(Value::as_u64).unwrap_or(100).clamp(1, 500) as usize;
        let filter = query.get("filter").and_then(Value::as_str).unwrap_or("").to_lowercase();
        let matches: Vec<_> = self.ui.sems.iter().filter(|s| filter.is_empty() || s.label.as_deref().unwrap_or("").to_lowercase().contains(&filter)).collect();
        let widgets = matches.iter().skip(offset).take(limit).map(|s| Value::object([
            ("id", Value::u64(s.id as u64)), ("window", Value::u64(s.win as u64)),
            ("kind", Value::u64(s.kind as u64)), ("label", s.label.as_ref().map(|s| Value::string(s.clone())).unwrap_or(Value::Null)),
            ("value", Value::i64(s.value as i64)), ("state", Value::u64(s.state as u64)), ("layer", Value::u64(s.layer as u64)),
            ("rect", Value::Array(s.rect.iter().map(|v| Value::i64(*v as i64)).collect())),
        ])).collect();
        let commands = self.ui.desc.cmds.iter().map(|c| Value::object([("id", Value::u64(c.id as u64)), ("label", Value::string(c.label)), ("enabled", Value::Bool(self.ui.cmd_flags(c.id) & CF_DISABLED == 0))])).collect();
        Ok(Value::object([
            ("revision", Value::u64(revision)), ("uiRevision", Value::u64(self.ui.frame as u64)),
            ("title", Value::string(self.ui.title())), ("focus", Value::u64(self.ui.focused() as u64)),
            ("modal", Value::u64(self.ui.modal_active() as u64)), ("capabilities", Value::u64(self.capabilities())),
            ("control", Value::object([("enabled", Value::Bool(self.ui.agent.enabled)), ("connected", Value::Bool(self.ui.agent.connected))])),
            ("widgets", Value::Array(widgets)), ("total", Value::u64(matches.len() as u64)),
            ("nextOffset", if offset.saturating_add(limit) < matches.len() { Value::u64((offset + limit) as u64) } else { Value::Null }),
            ("commands", Value::Array(commands)),
        ]))
    }
    #[cfg(any(target_arch = "wasm32", target_os = "android"))]
    fn task_status(&self, id: u64) -> Result<Value, ActionError> {
        use plat::web::RequestStatus as S;
        let state = u32::try_from(id).ok().and_then(plat::web::request_status).ok_or_else(|| error(ErrorCode::NotFound, "Task not found"))?;
        let status = match state { S::Pending => "running", S::Durable | S::Exported | S::Completed => "complete", S::Cancelled => "cancelled", _ => "failed" };
        Ok(Value::object([
            ("id", Value::u64(id)), ("revision", Value::u64(self.revision())), ("status", Value::string(status)), ("cancellable", Value::Bool(false)),
            ("storage", Value::string(match state { S::Durable => "durable", S::Exported => "exported", S::Pending => "pending", S::Quota => "quota_exceeded", S::Denied => "access_denied", S::Unsupported => "unsupported", S::Cancelled => "cancelled", S::Failed => "failed", S::Completed => "completed" })),
        ]))
    }
    fn resource_read(&self, uri: &str, offset: u64, limit: usize) -> Result<Vec<u8>, ActionError> {
        let Some((_, bytes)) = self.ui.agent.images.iter().find(|(name, _)| name == uri) else { return self.app.action_resource_read(uri, offset, limit.min(262_144)); };
        let start = usize::try_from(offset).ok().filter(|start| *start <= bytes.len()).ok_or_else(|| error(ErrorCode::InvalidInput, "Resource offset is out of range"))?;
        Ok(bytes[start..start.saturating_add(limit.min(262_144)).min(bytes.len())].to_vec())
    }
    fn resource_info(&self, uri: &str) -> Result<Resource, ActionError> {
        let Some((_, bytes)) = self.ui.agent.images.iter().find(|(name, _)| name == uri) else { return self.app.action_resource_info(uri); };
        Ok(Resource { uri: String::from(uri), name: String::from("Application screenshot"), mime_type: String::from("image/png"), size: Some(bytes.len() as u64) })
    }
}
fn error(code: ErrorCode, text: &str) -> ActionError { ActionError::new(code, text) }
fn number(input: &Value, key: &str, default: u32) -> Result<u32, ActionError> {
    input.get(key).map_or(Ok(default), |v| v.as_u64().and_then(|v| u32::try_from(v).ok()).ok_or_else(|| error(ErrorCode::InvalidInput, "Integer is outside the supported range")))
}

impl Ui {
    /// Whether external control was explicitly activated. No endpoint opens by reading this.
    pub fn agent_enabled(&self) -> bool { self.agent.enabled }
    /// Whether the active control session has received a client request.
    pub fn agent_connected(&self) -> bool { self.agent.connected }
    /// Enable or revoke external control. Revocation also discards screenshot resources.
    pub fn set_agent_enabled(&mut self, enabled: bool) {
        if enabled == self.agent.enabled { return; }
        self.agent.enabled = enabled;
        #[cfg(any(target_arch = "wasm32", target_os = "android"))]
        plat::web::set_agent_enabled(enabled);
        #[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
        if enabled && !self.headless {
            match agent_io::ipc::Server::bind(self.desc.title, "1", agent_core::Protocol::Json) {
                Ok(server) => { self.agent.server = Some(server); }
                Err(code) => {
                    self.agent.enabled = false;
                    plat::write_stderr(format!("agent endpoint could not start ({code})\n").as_bytes());
                }
            }
        }
        if !enabled {
            self.agent.connected = false;
            self.agent.images.clear();
            #[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
            { self.agent.server = None; }
        }
        self.invalidate();
    }
    #[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
    pub(crate) fn agent_poll(&mut self, app: &mut dyn App) {
        let Some(mut server) = self.agent.server.take() else { return; };
        let mut dispatcher = Dispatcher::new();
        dispatcher.set_enabled(self.agent.enabled);
        let changed = server.poll(&dispatcher, &mut UiHost::new(self, app));
        let connection_changed = self.agent.connected != (server.connected() != 0);
        self.agent.connected = server.connected() != 0;
        if self.agent.enabled { self.agent.server = Some(server); }
        if changed || connection_changed { self.invalidate(); }
    }
    pub(crate) fn agent_request(&mut self, app: &mut dyn App, session: &mut agent_core::RpcSession, line: &[u8]) -> Option<String> {
        let mut dispatcher = Dispatcher::new();
        dispatcher.set_enabled(self.agent.enabled);
        self.agent.connected = self.agent.enabled;
        session.handle(&dispatcher, &mut UiHost::new(self, app), line)
    }
}
impl Runtime {
    /// Explicitly opt into the same control dispatcher used by native JSON/MCP sessions.
    pub fn set_agent_enabled(&mut self, enabled: bool) { self.ui.set_agent_enabled(enabled && !self.is_finished()); }
    /// Handle one control request without IO. The caller owns one session per connection.
    pub fn control(&mut self, session: &mut agent_core::RpcSession, line: &[u8]) -> Option<String> {
        if self.is_finished() { self.ui.set_agent_enabled(false); }
        self.ui.agent_request(self.app.as_mut(), session, line)
    }
}

#[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
pub(crate) fn serve(ui: &mut Ui, app: &mut dyn App, protocol: agent_core::Protocol) -> i32 {
    ui.set_agent_enabled(true);
    ui.run_passes(app);
    let mut session = agent_core::RpcSession::new(protocol, ui.desc.title, "1");
    let code = agent_io::serve_stdio(|line| ui.agent_request(app, &mut session, line));
    ui.set_agent_enabled(false);
    app.command(ui, UIC_EXIT, 0);
    code
}

#[cfg(any(target_arch = "wasm32", target_os = "android"))]
pub(crate) fn serve(_ui: &mut Ui, _app: &mut dyn App, _protocol: agent_core::Protocol) -> i32 { 2 }
