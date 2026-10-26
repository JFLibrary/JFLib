// Shared semantic action dispatch. GUI adapters and all external transports call this core.
// New library code, independent of native windows, document types and language models.

use crate::{json::Value, schema::Schema};
use alloc::{collections::BTreeSet, format, string::String, vec::Vec};

/// Stable structured failure categories.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorCode {
    /// External control has not been explicitly activated.
    Disabled,
    /// The action or resource is unknown.
    NotFound,
    /// Parameters fail their declared schema or bounds.
    InvalidInput,
    /// The platform lacks a required capability.
    CapabilityUnavailable,
    /// The action is temporarily unavailable (including modal UI states).
    Unavailable,
    /// The caller's expected revision differs from the current revision.
    RevisionConflict,
    /// An operation or atomic transaction is unsupported.
    Unsupported,
    /// A bounded collection or response limit was exceeded.
    LimitExceeded,
    /// A task was cancelled at a defined interruption point.
    Cancelled,
    /// The host violated its action contract or returned an internal failure.
    Internal,
    /// The application or the user does not allow this action for external control.
    Forbidden,
    /// The user was asked to confirm the action and has not answered yet; retry later.
    ConfirmationRequired,
}

impl ErrorCode {
    /// Stable wire spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Disabled => "control_disabled",
            Self::NotFound => "not_found",
            Self::InvalidInput => "invalid_input",
            Self::CapabilityUnavailable => "capability_unavailable",
            Self::Unavailable => "unavailable",
            Self::RevisionConflict => "revision_conflict",
            Self::Unsupported => "unsupported",
            Self::LimitExceeded => "limit_exceeded",
            Self::Cancelled => "cancelled",
            Self::Internal => "internal_error",
            Self::Forbidden => "forbidden",
            Self::ConfirmationRequired => "confirmation_required",
        }
    }
}

/// Who may run an action through external control (agents, scripts over IPC / MCP).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    /// External calls run after the usual checks.
    Open,
    /// The host asks the user before an external call runs (`Host::confirm`).
    Confirm,
    /// Only trusted in-process GUI calls; external calls fail with `forbidden`.
    Local,
}

impl Access {
    /// Stable wire spelling ("open", "confirm", "local").
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Confirm => "confirm",
            Self::Local => "local",
        }
    }
}

/// An actionable failure with structured details; no transport-specific error numbers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActionError {
    /// Stable error category.
    pub code: ErrorCode,
    /// Human-readable diagnosis.
    pub message: String,
    /// Structured correction details such as a field pointer or current revision.
    pub details: Value,
}

impl ActionError {
    /// Construct a failure without extra details.
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            details: Value::Null,
        }
    }
    /// Attach machine-readable context.
    pub fn with_details(mut self, details: Value) -> Self {
        self.details = details;
        self
    }
    /// JSON form shared by batch, JSON-RPC and MCP results.
    pub fn to_json(&self) -> Value {
        Value::object([
            ("code", Value::string(self.code.as_str())),
            ("message", Value::string(&self.message)),
            ("details", self.details.clone()),
        ])
    }
}

/// A static semantic action contract. Register each implementation once in its host.
#[derive(Clone, Copy, Debug)]
pub struct ActionSpec {
    /// Stable, language-independent identifier, e.g. `document.text.replace`.
    pub name: &'static str,
    /// Description for human and automated discovery.
    pub description: &'static str,
    /// Validated input schema.
    pub input: &'static Schema,
    /// Validated completed-result schema.
    pub output: &'static Schema,
    /// Bit set of platform services required by this action; defined by the platform layer.
    pub capabilities: u64,
    /// Whether the action can change application or document state.
    pub mutation: bool,
    /// Require an expected revision for external mutation calls.
    pub revision_required: bool,
    /// Whether the operation is integrated with the application's undo history.
    pub undo: bool,
    /// Transaction domain, or None when atomic batches are unsupported.
    pub transaction: Option<&'static str>,
    /// Whether the operation can return a cancellable task.
    pub cancellable: bool,
    /// Optional compatible numeric GUI command identifier.
    pub command_id: Option<u32>,
    /// Who may run it through external control; hosts may tighten it (`Host::access`).
    pub access: Access,
}

impl ActionSpec {
    /// Parameterless read action with a general structured-object result.
    pub const fn read(name: &'static str, description: &'static str) -> Self {
        Self {
            name,
            description,
            input: &crate::schema::EMPTY_OBJECT,
            output: &crate::schema::OBJECT,
            capabilities: 0,
            mutation: false,
            revision_required: false,
            undo: false,
            transaction: None,
            cancellable: false,
            command_id: None,
            access: Access::Open,
        }
    }
    /// External calls need the user's confirmation first.
    pub const fn confirm(mut self) -> Self {
        self.access = Access::Confirm;
        self
    }
    /// Only trusted in-process GUI calls; external control is refused.
    pub const fn local_only(mut self) -> Self {
        self.access = Access::Local;
        self
    }
    /// Revision-checked mutation; the caller can set undo and transaction metadata.
    pub const fn write(name: &'static str, description: &'static str) -> Self {
        Self {
            mutation: true,
            revision_required: true,
            ..Self::read(name, description)
        }
    }
    /// Replace both schemas while preserving other metadata.
    pub const fn schemas(mut self, input: &'static Schema, output: &'static Schema) -> Self {
        self.input = input;
        self.output = output;
        self
    }
}

/// Request metadata passed unchanged to registered handlers.
#[derive(Clone, Copy, Debug, Default)]
pub struct ActionContext {
    /// Expected revision supplied by the caller and checked by the dispatcher.
    pub expected_revision: Option<u64>,
    /// True when executing inside a host transaction.
    pub atomic: bool,
    /// True for trusted in-process GUI commands, which may omit an expected revision.
    pub local: bool,
}

/// Small metadata link to binary/file data, kept out of state snapshots.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resource {
    /// Opaque application resource identifier.
    pub uri: String,
    /// Friendly name.
    pub name: String,
    /// Content media type.
    pub mime_type: String,
    /// Exact payload byte count if known.
    pub size: Option<u64>,
}

impl Resource {
    /// Metadata only; resource bytes are read separately.
    pub fn to_json(&self) -> Value {
        Value::object([
            ("uri", Value::string(&self.uri)),
            ("name", Value::string(&self.name)),
            ("mimeType", Value::string(&self.mime_type)),
            ("size", self.size.map(Value::u64).unwrap_or(Value::Null)),
        ])
    }
}

/// Action completion or a deferred host-managed task.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActionResult {
    /// Completed structured output (Null for deferred work).
    pub value: Value,
    /// Current host revision after execution.
    pub revision: u64,
    /// Task identifier when execution continues asynchronously.
    pub task: Option<u64>,
    /// Metadata for optional separately readable payloads.
    pub resources: Vec<Resource>,
}

impl ActionResult {
    /// Return a completed result.
    pub fn completed(value: Value, revision: u64) -> Self {
        Self {
            value,
            revision,
            task: None,
            resources: Vec::new(),
        }
    }
    /// Return a task whose state can be queried or cancelled.
    pub fn deferred(task: u64, revision: u64) -> Self {
        Self {
            value: Value::Null,
            revision,
            task: Some(task),
            resources: Vec::new(),
        }
    }
    /// Shared structured wire response.
    pub fn to_json(&self) -> Value {
        Value::object([
            (
                "status",
                Value::string(if self.task.is_some() {
                    "pending"
                } else {
                    "complete"
                }),
            ),
            ("revision", Value::u64(self.revision)),
            ("value", self.value.clone()),
            ("task", self.task.map(Value::u64).unwrap_or(Value::Null)),
            (
                "resources",
                Value::Array(self.resources.iter().map(Resource::to_json).collect()),
            ),
        ])
    }
}

/// Application adapter; the dispatcher never invents domain semantics from pixels.
pub trait Host {
    /// All registered action contracts, normally a static slice.
    fn actions(&self) -> &[ActionSpec];
    /// Current document/application revision for optimistic concurrency control.
    fn revision(&self) -> u64;
    /// Services currently available from the native or browser platform.
    fn capabilities(&self) -> u64;
    /// Check dynamic availability, including modal state, selection and permissions.
    fn availability(&self, _action: &ActionSpec) -> Result<(), ActionError> {
        Ok(())
    }
    /// Who may run the action externally: its `ActionSpec::access`, or stricter (the app's
    /// permission manifest).
    fn access(&self, action: &ActionSpec) -> Access {
        action.access
    }
    /// Ask the user before an external call of an `Access::Confirm` action runs. `Ok` lets it
    /// run; `forbidden` when refused, `confirmation_required` while the question is open.
    fn confirm(&mut self, _action: &ActionSpec, _input: &Value) -> Result<(), ActionError> {
        Err(ActionError::new(
            ErrorCode::Forbidden,
            "This action needs the user's confirmation, which this host cannot ask for",
        )
        .with_details(Value::object([("reason", Value::string("unattended"))])))
    }
    /// An external call of an `Access::Local` action was refused (an audit hook).
    fn refused(&mut self, _action: &ActionSpec) {}
    /// Execute the same handler used by GUI/keyboard command adapters.
    fn execute(
        &mut self,
        action: &ActionSpec,
        input: &Value,
        context: &ActionContext,
    ) -> Result<ActionResult, ActionError>;
    /// Whether a named transaction domain supports reliable rollback.
    fn supports_transaction(&self, _domain: &str) -> bool {
        false
    }
    /// Begin one undoable transaction. Atomic actions must complete synchronously.
    fn begin_transaction(&mut self, _domain: &str) -> Result<(), ActionError> {
        Err(ActionError::new(
            ErrorCode::Unsupported,
            "Transactions are unavailable",
        ))
    }
    /// Commit all changes in the active transaction.
    fn commit_transaction(&mut self) -> Result<(), ActionError> {
        Err(ActionError::new(
            ErrorCode::Unsupported,
            "Transactions are unavailable",
        ))
    }
    /// Restore state and revision to before begin_transaction.
    fn rollback_transaction(&mut self) -> Result<(), ActionError> {
        Err(ActionError::new(
            ErrorCode::Unsupported,
            "Transactions are unavailable",
        ))
    }
    /// Filtered, paginated semantic state; these control methods (inspect, tasks, resources)
    /// are not gated by `access`, so the host must apply its own access rules to them and
    /// should not hand out guessable task ids; hosts may accept a `sinceRevision` query.
    fn inspect(&self, _query: &Value) -> Result<Value, ActionError> {
        Err(ActionError::new(
            ErrorCode::Unsupported,
            "Inspection is unavailable",
        ))
    }
    /// Query a host-managed asynchronous task.
    fn task_status(&self, _id: u64) -> Result<Value, ActionError> {
        Err(ActionError::new(ErrorCode::NotFound, "Task not found"))
    }
    /// Request cancellation at the task's next defined interruption point.
    fn task_cancel(&mut self, _id: u64) -> Result<Value, ActionError> {
        Err(ActionError::new(ErrorCode::NotFound, "Task not found"))
    }
    /// Metadata for one application-owned resource; arbitrary filesystem paths are not URIs.
    fn resource_info(&self, _uri: &str) -> Result<Resource, ActionError> {
        Err(ActionError::new(ErrorCode::NotFound, "Resource not found"))
    }
    /// Read a bounded resource slice. Offsets are bytes; unknown URIs must be rejected.
    fn resource_read(
        &self,
        _uri: &str,
        _offset: u64,
        _limit: usize,
    ) -> Result<Vec<u8>, ActionError> {
        Err(ActionError::new(ErrorCode::NotFound, "Resource not found"))
    }
}

/// Borrowed single invocation.
pub struct Invocation<'a> {
    /// Stable action name.
    pub action: &'a str,
    /// Structured parameters.
    pub input: &'a Value,
    /// Expected revision for mutations.
    pub expected_revision: Option<u64>,
}

/// Owned invocation for an ordered batch.
#[derive(Clone, Debug)]
pub struct BatchCall {
    /// Stable action name.
    pub action: String,
    /// Structured parameters.
    pub input: Value,
    /// Revision at the moment this call should execute; atomic calls refer to the batch start.
    pub expected_revision: Option<u64>,
}

/// Ordered batch outcome, retaining the outcome of each attempted call.
#[derive(Clone, Debug)]
pub struct BatchResult {
    /// Per-call results in original order.
    pub results: Vec<Result<ActionResult, ActionError>>,
    /// True if a failed atomic batch was rolled back successfully.
    pub rolled_back: bool,
    /// Current revision, including rollback.
    pub revision: u64,
}

impl BatchResult {
    /// Structured batch result.
    pub fn to_json(&self) -> Value {
        Value::object([
            (
                "results",
                Value::Array(
                    self.results
                        .iter()
                        .map(|r| match r {
                            Ok(v) => {
                                Value::object([("ok", Value::Bool(true)), ("result", v.to_json())])
                            }
                            Err(e) => {
                                Value::object([("ok", Value::Bool(false)), ("error", e.to_json())])
                            }
                        })
                        .collect(),
                ),
            ),
            ("rolledBack", Value::Bool(self.rolled_back)),
            ("revision", Value::u64(self.revision)),
        ])
    }
}

/// One deterministic page of action discovery.
#[derive(Clone, Debug)]
pub struct Page {
    /// Metadata of matching actions, without full schemas.
    pub actions: Vec<Value>,
    /// Exclusive action-name cursor for the next page.
    pub next_cursor: Option<String>,
    /// Current state revision.
    pub revision: u64,
}

impl Page {
    /// Shared discovery response.
    pub fn to_json(&self) -> Value {
        Value::object([
            ("actions", Value::Array(self.actions.clone())),
            (
                "nextCursor",
                self.next_cursor
                    .as_ref()
                    .map(|s| Value::string(s))
                    .unwrap_or(Value::Null),
            ),
            ("revision", Value::u64(self.revision)),
        ])
    }
}

/// Activation gate and deterministic dispatcher. Opening transports remains a host decision.
#[derive(Default)]
pub struct Dispatcher {
    enabled: bool,
}

impl Dispatcher {
    /// Construct with all external access disabled.
    pub const fn new() -> Self {
        Self { enabled: false }
    }
    /// Explicitly enable/disable control; transports should also close when disabled.
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }
    /// Whether external access was explicitly enabled.
    pub fn enabled(&self) -> bool {
        self.enabled
    }
    /// Reject access before consulting application state.
    pub fn check_enabled(&self) -> Result<(), ActionError> {
        if self.enabled {
            Ok(())
        } else {
            Err(ActionError::new(
                ErrorCode::Disabled,
                "External control is disabled",
            ))
        }
    }
    fn find(&self, host: &impl Host, name: &str) -> Result<ActionSpec, ActionError> {
        validate_registry(host.actions())?;
        host.actions()
            .iter()
            .find(|action| action.name == name)
            .copied()
            .ok_or_else(|| {
                ActionError::new(ErrorCode::NotFound, "Action not found")
                    .with_details(Value::object([("action", Value::string(name))]))
            })
    }
    fn ready(&self, host: &impl Host, action: &ActionSpec) -> Result<(), ActionError> {
        let missing = action.capabilities & !host.capabilities();
        if missing != 0 {
            return Err(ActionError::new(
                ErrorCode::CapabilityUnavailable,
                "Required platform services are unavailable",
            )
            .with_details(Value::object([(
                "missingCapabilities",
                Value::u64(missing),
            )])));
        }
        host.availability(action)
    }
    fn validate(
        &self,
        host: &impl Host,
        action: &ActionSpec,
        input: &Value,
        context: &ActionContext,
        revision: u64,
    ) -> Result<(), ActionError> {
        if let Err(e) = action.input.validate(input) {
            return Err(ActionError::new(
                ErrorCode::InvalidInput,
                "Action input does not match its schema",
            )
            .with_details(Value::object([
                ("path", Value::string(e.path)),
                ("reason", Value::string(e.reason)),
            ])));
        }
        self.ready(host, action)?;
        if action.mutation
            && action.revision_required
            && !context.local
            && context.expected_revision.is_none()
        {
            return Err(ActionError::new(
                ErrorCode::InvalidInput,
                "This mutation requires expectedRevision",
            ));
        }
        if action.mutation
            && context
                .expected_revision
                .is_some_and(|expected| expected != revision)
        {
            return Err(ActionError::new(
                ErrorCode::RevisionConflict,
                "The application state changed after it was inspected",
            )
            .with_details(Value::object([
                (
                    "expectedRevision",
                    context
                        .expected_revision
                        .map(Value::u64)
                        .unwrap_or(Value::Null),
                ),
                ("currentRevision", Value::u64(revision)),
            ])));
        }
        Ok(())
    }
    fn invoke(
        &self,
        host: &mut impl Host,
        action: &ActionSpec,
        input: &Value,
        context: &ActionContext,
    ) -> Result<ActionResult, ActionError> {
        let mut result = host.execute(action, input, context)?;
        result.revision = host.revision();
        if context.atomic && result.task.is_some() {
            return Err(ActionError::new(
                ErrorCode::Internal,
                "An atomic action returned deferred work",
            ));
        }
        if result.task.is_none() {
            if let Err(e) = action.output.validate(&result.value) {
                return Err(ActionError::new(
                    ErrorCode::Internal,
                    "The action returned an invalid result",
                )
                .with_details(Value::object([
                    ("path", Value::string(e.path)),
                    ("reason", Value::string(e.reason)),
                ])));
            }
        }
        Ok(result)
    }
    /// Execute an enabled external action after schema, capability and revision checks.
    pub fn execute(
        &self,
        host: &mut impl Host,
        call: Invocation<'_>,
    ) -> Result<ActionResult, ActionError> {
        self.check_enabled()?;
        let action = self.find(host, call.action)?;
        let context = ActionContext {
            expected_revision: call.expected_revision,
            atomic: false,
            local: false,
        };
        let access = host.access(&action);
        // a Local action reveals nothing about its schema or availability to external callers
        if matches!(access, Access::Local) {
            self.consent(host, &action, call.input)?;
        }
        self.validate(host, &action, call.input, &context, host.revision())?;
        if !matches!(access, Access::Local) {
            self.consent(host, &action, call.input)?;
            if matches!(access, Access::Confirm) {
                // the document may have changed while the user was asked
                self.validate(host, &action, call.input, &context, host.revision())?;
            }
        }
        self.invoke(host, &action, call.input, &context)
    }
    // An external call: the action's access (Local is refused, Confirm asks the host's user).
    fn consent(
        &self,
        host: &mut impl Host,
        action: &ActionSpec,
        input: &Value,
    ) -> Result<(), ActionError> {
        match host.access(action) {
            Access::Open => Ok(()),
            Access::Local => {
                host.refused(action);
                Err(ActionError::new(
                    ErrorCode::Forbidden,
                    "This action is not available to external control",
                )
                .with_details(Value::object([
                    ("action", Value::string(action.name)),
                    ("reason", Value::string("app")),
                ])))
            }
            Access::Confirm => host.confirm(action, input),
        }
    }
    /// Invoke a trusted GUI/shortcut command through the same schema and handler.
    pub fn execute_local(
        &self,
        host: &mut impl Host,
        name: &str,
        input: &Value,
    ) -> Result<ActionResult, ActionError> {
        let action = self.find(host, name)?;
        let context = ActionContext {
            local: true,
            ..ActionContext::default()
        };
        self.validate(host, &action, input, &context, host.revision())?;
        self.invoke(host, &action, input, &context)
    }
    /// Resolve a legacy numeric GUI command to its registered semantic handler.
    pub fn execute_command(
        &self,
        host: &mut impl Host,
        id: u32,
        input: &Value,
    ) -> Result<ActionResult, ActionError> {
        validate_registry(host.actions())?;
        let name = host
            .actions()
            .iter()
            .find(|a| a.command_id == Some(id))
            .map(|a| a.name)
            .ok_or_else(|| {
                ActionError::new(ErrorCode::NotFound, "GUI command is not registered")
            })?;
        self.execute_local(host, name, input)
    }
    /// Run an ordered batch. Atomic support and every static precondition are checked first.
    pub fn batch(
        &self,
        host: &mut impl Host,
        calls: &[BatchCall],
        atomic: bool,
        stop_on_error: bool,
    ) -> Result<BatchResult, ActionError> {
        self.check_enabled()?;
        if calls.is_empty() || calls.len() > 128 {
            return Err(ActionError::new(
                ErrorCode::InvalidInput,
                "A batch must contain 1 to 128 calls",
            ));
        }
        let start_revision = host.revision();
        let mut prepared = Vec::new();
        if atomic {
            let mut domain = None;
            for call in calls {
                let action = self.find(host, &call.action)?;
                let Some(next) = action.transaction else {
                    return Err(ActionError::new(
                        ErrorCode::Unsupported,
                        "An action does not support atomic batches",
                    )
                    .with_details(Value::object([("action", Value::string(&call.action))])));
                };
                if domain.is_some_and(|d| d != next) || !host.supports_transaction(next) {
                    return Err(ActionError::new(
                        ErrorCode::Unsupported,
                        "The batch has no shared supported transaction domain",
                    ));
                }
                domain = Some(next);
                let context = ActionContext {
                    expected_revision: call.expected_revision,
                    atomic: true,
                    local: false,
                };
                if matches!(host.access(&action), Access::Local) {
                    self.consent(host, &action, &call.input)?;
                }
                self.validate(host, &action, &call.input, &context, start_revision)?;
                self.consent(host, &action, &call.input)?;
                prepared.push(action);
            }
            host.begin_transaction(domain.unwrap_or(""))?;
        }
        let mut results = Vec::with_capacity(calls.len());
        for (index, call) in calls.iter().enumerate() {
            let result = if atomic {
                let action = prepared[index];
                let context = ActionContext {
                    expected_revision: call.expected_revision,
                    atomic: true,
                    local: false,
                };
                self.ready(host, &action)
                    .and_then(|()| self.invoke(host, &action, &call.input, &context))
            } else {
                self.execute(
                    host,
                    Invocation {
                        action: &call.action,
                        input: &call.input,
                        expected_revision: call.expected_revision,
                    },
                )
            };
            let failed = result.is_err();
            results.push(result);
            if failed && atomic {
                host.rollback_transaction().map_err(|e| {
                    ActionError::new(ErrorCode::Internal, "Atomic rollback failed")
                        .with_details(e.to_json())
                })?;
                return Ok(BatchResult {
                    results,
                    rolled_back: true,
                    revision: host.revision(),
                });
            }
            if failed && stop_on_error {
                break;
            }
        }
        if atomic {
            if let Err(error) = host.commit_transaction() {
                host.rollback_transaction().map_err(|e| {
                    ActionError::new(ErrorCode::Internal, "Rollback after commit failure failed")
                        .with_details(e.to_json())
                })?;
                return Err(error);
            }
        }
        Ok(BatchResult {
            results,
            rolled_back: false,
            revision: host.revision(),
        })
    }
    /// Find actions by name/description, with stable lexical ordering and name cursors.
    pub fn discover(
        &self,
        host: &impl Host,
        query: &str,
        after: Option<&str>,
        limit: usize,
    ) -> Result<Page, ActionError> {
        self.check_enabled()?;
        validate_registry(host.actions())?;
        if limit == 0 || limit > 100 || query.len() > 256 {
            return Err(ActionError::new(
                ErrorCode::InvalidInput,
                "Discovery limit is 1 to 100 and query is at most 256 bytes",
            ));
        }
        let query = query.to_lowercase();
        let mut actions: Vec<&ActionSpec> = host
            .actions()
            .iter()
            .filter(|a| {
                (query.is_empty()
                    || a.name.to_lowercase().contains(&query)
                    || a.description.to_lowercase().contains(&query))
                    && after.is_none_or(|cursor| a.name > cursor)
            })
            .collect();
        actions.sort_by_key(|a| a.name);
        let next_cursor = if actions.len() > limit {
            Some(actions[limit - 1].name.into())
        } else {
            None
        };
        actions.truncate(limit);
        Ok(Page {
            actions: actions
                .into_iter()
                .map(|a| self.metadata(host, a, false))
                .collect(),
            next_cursor,
            revision: host.revision(),
        })
    }
    /// Return schemas, behavior metadata and a concrete availability reason for one action.
    pub fn describe(&self, host: &impl Host, name: &str) -> Result<Value, ActionError> {
        self.check_enabled()?;
        let action = self.find(host, name)?;
        Ok(self.metadata(host, &action, true))
    }
    fn metadata(&self, host: &impl Host, action: &ActionSpec, schemas: bool) -> Value {
        let availability = self.ready(host, action);
        let mut value = Value::object([
            ("name", Value::string(action.name)),
            ("description", Value::string(action.description)),
            ("available", Value::Bool(availability.is_ok())),
            (
                "unavailableReason",
                availability
                    .err()
                    .map(|e| e.to_json())
                    .unwrap_or(Value::Null),
            ),
            ("capabilities", Value::u64(action.capabilities)),
            ("mutation", Value::Bool(action.mutation)),
            ("revisionRequired", Value::Bool(action.revision_required)),
            ("undo", Value::Bool(action.undo)),
            (
                "transaction",
                action.transaction.map(Value::string).unwrap_or(Value::Null),
            ),
            ("cancellable", Value::Bool(action.cancellable)),
            ("access", Value::string(host.access(action).as_str())),
            (
                "commandId",
                action
                    .command_id
                    .map(|id| Value::u64(id as u64))
                    .unwrap_or(Value::Null),
            ),
        ]);
        if schemas {
            value.insert("inputSchema", action.input.to_json());
            value.insert("outputSchema", action.output.to_json());
        }
        value
    }
}

/// Validate stable identifiers and prevent ambiguous name/command registrations.
pub fn validate_registry(actions: &[ActionSpec]) -> Result<(), ActionError> {
    if actions.len() > 4096 {
        return Err(ActionError::new(
            ErrorCode::LimitExceeded,
            "Too many registered actions",
        ));
    }
    let mut names = BTreeSet::new();
    let mut commands = BTreeSet::new();
    for action in actions.iter() {
        if action.name.is_empty()
            || action.name.len() > 128
            || !action
                .name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
        {
            return Err(ActionError::new(
                ErrorCode::Internal,
                "Invalid registered action name",
            ));
        }
        if !names.insert(action.name)
            || action.command_id.is_some_and(|id| !commands.insert(id))
        {
            return Err(ActionError::new(
                ErrorCode::Internal,
                format!("Duplicate action or command: {}", action.name),
            ));
        }
        if action.revision_required && !action.mutation {
            return Err(ActionError::new(
                ErrorCode::Internal,
                "A read action cannot require a mutation revision",
            ));
        }
    }
    Ok(())
}
