// Versioned newline JSON-RPC plus MCP adapters. No I/O: transports supply complete lines.
// New shared implementation, kept independent from OS streams and browser MessagePorts.

use crate::{json, ActionError, BatchCall, Dispatcher, ErrorCode, Host, Invocation, Value};
use alloc::{string::String, vec::Vec};

/// Application protocol revision carried by JSON control responses.
pub const JSON_VERSION: &str = "1";
/// Current MCP revision, selected per request through params._meta.
pub const MCP_VERSION: &str = "2026-07-28";
/// Explicitly supported initialization-based MCP revision.
pub const MCP_LEGACY_VERSION: &str = "2025-11-25";

/// One transport chooses its protocol explicitly; there is no guessing from requests.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Protocol {
    /// jfguiLib JSON-RPC control protocol version 1.
    Json,
    /// MCP with current per-request metadata or the supported legacy handshake.
    Mcp,
}

/// Select a connection's protocol from its first valid JSON-RPC envelope. An explicit
/// modern MCP version or legacy initialization selects MCP; other requests select JSON v1.
/// Malformed envelopes return None so transports can report them without locking the session.
pub fn requested_protocol(line: &[u8]) -> Option<Protocol> {
    let request = json::parse(line).ok()?;
    if request.get("jsonrpc").and_then(Value::as_str) != Some("2.0")
        || request.get("method").and_then(Value::as_str).is_none()
        || request.get("result").is_some()
        || request.get("error").is_some()
        || request
            .get("params")
            .is_some_and(|params| params.as_object().is_none())
        || request.get("id").is_some_and(|id| {
            !matches!(id, Value::Null | Value::String(_))
                && id.as_u64().is_none()
                && id.as_i64().is_none()
        })
    {
        return None;
    }
    let is_mcp = request.get("params").is_some_and(|params| {
        params
            .get("_meta")
            .and_then(|meta| meta.get("io.modelcontextprotocol/protocolVersion"))
            .is_some()
            || request.get("method").and_then(Value::as_str) == Some("initialize")
                && params.get("protocolVersion").is_some()
    });
    Some(if is_mcp {
        Protocol::Mcp
    } else {
        Protocol::Json
    })
}

/// Per-connection protocol framing state; application state stays in the Host.
pub struct RpcSession {
    pub(crate) protocol: Protocol,
    pub(crate) server_name: String,
    pub(crate) server_version: String,
    pub(crate) legacy_initialized: bool,
    pub(crate) legacy_ready: bool,
}

impl RpcSession {
    /// Whether initialization-based MCP negotiation must survive between requests.
    /// Stateless JSON/current-MCP browser sessions may be evicted when an inactive cache fills.
    pub fn requires_persistent_session(&self) -> bool {
        self.legacy_initialized
    }
    /// Create a protocol adapter without activating application control.
    pub fn new(protocol: Protocol, server_name: &str, server_version: &str) -> Self {
        Self {
            protocol,
            server_name: server_name.into(),
            server_version: server_version.into(),
            legacy_initialized: false,
            legacy_ready: false,
        }
    }
    /// Handle one UTF-8 JSON line, returning a newline-terminated response or no notification reply.
    /// This method never logs or writes standard streams. Use one session per connection.
    pub fn handle(
        &mut self,
        dispatcher: &Dispatcher,
        host: &mut impl Host,
        line: &[u8],
    ) -> Option<String> {
        let request = match json::parse(line) {
            Ok(value) => value,
            Err(error) => {
                return Some(encode(error_response(
                    Value::Null,
                    RpcError {
                        code: -32700,
                        message: "Invalid JSON".into(),
                        data: Value::object([
                            ("offset", Value::u64(error.offset as u64)),
                            ("reason", Value::string(error.kind)),
                        ]),
                    },
                )))
            }
        };
        let id = request.get("id").cloned();
        let valid_id = id.as_ref().is_none_or(|id| {
            matches!(id, Value::String(_))
                || id.as_i64().is_some()
                || id.as_u64().is_some()
                || self.protocol == Protocol::Json && matches!(id, Value::Null)
        });
        if request.as_object().is_none()
            || request.get("jsonrpc").and_then(Value::as_str) != Some("2.0")
            || request.get("method").and_then(Value::as_str).is_none()
            || !valid_id
            || request.get("result").is_some()
            || request.get("error").is_some()
        {
            return Some(encode(error_response(
                Value::Null,
                RpcError::new(-32600, "Invalid JSON-RPC request"),
            )));
        }
        let method = request.get("method").and_then(Value::as_str).unwrap_or("");
        let empty = Value::object([]);
        let params = request.get("params").unwrap_or(&empty);
        if params.as_object().is_none() {
            return id.map(|id| {
                encode(error_response(
                    id,
                    RpcError::new(-32602, "Parameters must be an object"),
                ))
            });
        }
        if id.is_none() {
            // Deliberately accept only recognized notifications; semantic writes require IDs.
            if dispatcher.enabled()
                && self.protocol == Protocol::Mcp
                && method == "notifications/initialized"
                && self.legacy_initialized
            {
                self.legacy_ready = true;
            }
            // All handlers here finish synchronously or return an explicit app task ID.
            // An MCP cancellation for an already completed request therefore has no effect.
            return None;
        }
        let id = id.unwrap_or(Value::Null);
        let result = dispatcher
            .check_enabled()
            .map_err(RpcError::action)
            .and_then(|()| match self.protocol {
                Protocol::Json => self.json_request(dispatcher, host, method, params),
                Protocol::Mcp => self.mcp_request(dispatcher, host, method, params),
            });
        Some(encode(match result {
            Ok(value) => Value::object([
                ("jsonrpc", Value::string("2.0")),
                ("id", id),
                ("result", value),
            ]),
            Err(error) => error_response(id, error),
        }))
    }
    fn json_request(
        &self,
        dispatcher: &Dispatcher,
        host: &mut impl Host,
        method: &str,
        params: &Value,
    ) -> Result<Value, RpcError> {
        if params
            .get("protocolVersion")
            .is_some_and(|v| v.as_str() != Some(JSON_VERSION))
        {
            return Err(RpcError::new(
                -32602,
                "Unsupported control protocolVersion; expected 1",
            ));
        }
        let mut clean = params.clone();
        if let Value::Object(fields) = &mut clean {
            fields.retain(|(key, _)| key != "protocolVersion");
        }
        let mut result = if method == "agent/info" {
            allowed(&clean, &[]).map_err(RpcError::action)?;
            Value::object([
                ("protocolVersion", Value::string(JSON_VERSION)),
                ("serverInfo", self.server_info()),
                ("revision", Value::u64(host.revision())),
                ("capabilities", Value::u64(host.capabilities())),
                ("controlEnabled", Value::Bool(true)),
            ])
        } else {
            if !matches!(
                method,
                "actions/list"
                    | "actions/describe"
                    | "actions/execute"
                    | "actions/batch"
                    | "state/inspect"
                    | "tasks/get"
                    | "tasks/cancel"
                    | "resources/read"
                    | "view/render"
            ) {
                return Err(RpcError::new(-32601, "Method not found"));
            }
            control(dispatcher, host, method, &clean).map_err(RpcError::action)?
        };
        result.insert("protocolVersion", Value::string(JSON_VERSION));
        Ok(result)
    }
    pub(crate) fn server_info(&self) -> Value {
        Value::object([
            ("name", Value::string(&self.server_name)),
            ("version", Value::string(&self.server_version)),
        ])
    }
}

/// Execute the version-independent methods used by JSON and the MCP tool catalog.
pub fn control(
    dispatcher: &Dispatcher,
    host: &mut impl Host,
    method: &str,
    params: &Value,
) -> Result<Value, ActionError> {
    dispatcher.check_enabled()?;
    if params.as_object().is_none() {
        return Err(invalid("Parameters must be an object"));
    }
    match method {
        "actions/list" => {
            allowed(params, &["query", "cursor", "limit"])?;
            let query = optional_str(params, "query")?.unwrap_or("");
            let cursor = optional_str(params, "cursor")?;
            let limit = optional_u64(params, "limit")?.unwrap_or(30);
            if limit > 100 {
                return Err(invalid("Discovery limit must be at most 100"));
            }
            Ok(dispatcher
                .discover(host, query, cursor, limit as usize)?
                .to_json())
        }
        "actions/describe" => {
            allowed(params, &["action"])?;
            dispatcher.describe(host, required_str(params, "action")?)
        }
        "actions/execute" => {
            allowed(params, &["action", "input", "expectedRevision"])?;
            let empty = Value::object([]);
            let call = Invocation {
                action: required_str(params, "action")?,
                input: params.get("input").unwrap_or(&empty),
                expected_revision: optional_u64(params, "expectedRevision")?,
            };
            Ok(dispatcher.execute(host, call)?.to_json())
        }
        "actions/batch" => {
            allowed(params, &["calls", "atomic", "stopOnError"])?;
            let Some(calls) = params.get("calls").and_then(Value::as_array) else {
                return Err(invalid("calls must be an array"));
            };
            if calls.is_empty() || calls.len() > 128 {
                return Err(invalid("A batch must contain 1 to 128 calls"));
            }
            let mut parsed = Vec::with_capacity(calls.len());
            for call in calls {
                allowed(call, &["action", "input", "expectedRevision"])?;
                parsed.push(BatchCall {
                    action: required_str(call, "action")?.into(),
                    input: call
                        .get("input")
                        .cloned()
                        .unwrap_or_else(|| Value::object([])),
                    expected_revision: optional_u64(call, "expectedRevision")?,
                });
            }
            Ok(dispatcher
                .batch(
                    host,
                    &parsed,
                    optional_bool(params, "atomic")?.unwrap_or(false),
                    optional_bool(params, "stopOnError")?.unwrap_or(true),
                )?
                .to_json())
        }
        "state/inspect" => host.inspect(params),
        "tasks/get" => {
            allowed(params, &["id"])?;
            host.task_status(required_u64(params, "id")?)
        }
        "tasks/cancel" => {
            allowed(params, &["id"])?;
            host.task_cancel(required_u64(params, "id")?)
        }
        "resources/read" => {
            allowed(params, &["uri", "offset", "limit"])?;
            let uri = required_str(params, "uri")?;
            let offset = optional_u64(params, "offset")?.unwrap_or(0);
            let limit = optional_u64(params, "limit")?.unwrap_or(32_768);
            if limit == 0 || limit > 65_536 {
                return Err(invalid("Resource limit must be 1 to 65536 bytes"));
            }
            let info = host.resource_info(uri)?;
            if info.size.is_some_and(|size| offset > size) {
                return Err(invalid("Resource offset exceeds its size"));
            }
            let bytes = host.resource_read(uri, offset, limit as usize)?;
            if bytes.len() > limit as usize {
                return Err(ActionError::new(
                    ErrorCode::Internal,
                    "Resource reader exceeded its limit",
                ));
            }
            let next = offset
                .checked_add(bytes.len() as u64)
                .ok_or_else(|| invalid("Resource offset overflow"))?;
            let eof = info.size.is_some_and(|size| next >= size) || bytes.len() < limit as usize;
            Ok(Value::object([
                ("resource", info.to_json()),
                ("offset", Value::u64(offset)),
                ("nextOffset", Value::u64(next)),
                ("eof", Value::Bool(eof)),
                ("encoding", Value::string("base64")),
                ("data", Value::string(base64(&bytes))),
            ]))
        }
        "view/render" => {
            let call = Invocation {
                action: "ui.render",
                input: params,
                expected_revision: None,
            };
            Ok(dispatcher.execute(host, call)?.to_json())
        }
        _ => Err(ActionError::new(
            ErrorCode::NotFound,
            "Control method not found",
        )),
    }
}

pub(crate) fn required_str<'a>(value: &'a Value, key: &str) -> Result<&'a str, ActionError> {
    optional_str(value, key)?.ok_or_else(|| invalid("Required string parameter is missing"))
}
pub(crate) fn optional_str<'a>(
    value: &'a Value,
    key: &str,
) -> Result<Option<&'a str>, ActionError> {
    match value.get(key) {
        None => Ok(None),
        Some(Value::String(text)) if text.len() <= 4096 => Ok(Some(text)),
        _ => Err(invalid("Expected a string of at most 4096 bytes")),
    }
}
pub(crate) fn required_u64(value: &Value, key: &str) -> Result<u64, ActionError> {
    optional_u64(value, key)?.ok_or_else(|| invalid("Required integer parameter is missing"))
}
pub(crate) fn optional_u64(value: &Value, key: &str) -> Result<Option<u64>, ActionError> {
    match value.get(key) {
        None => Ok(None),
        Some(v) => v
            .as_u64()
            .map(Some)
            .ok_or_else(|| invalid("Expected an unsigned integer")),
    }
}
fn optional_bool(value: &Value, key: &str) -> Result<Option<bool>, ActionError> {
    match value.get(key) {
        None => Ok(None),
        Some(Value::Bool(v)) => Ok(Some(*v)),
        _ => Err(invalid("Expected a boolean")),
    }
}
pub(crate) fn allowed(value: &Value, names: &[&str]) -> Result<(), ActionError> {
    let Some(fields) = value.as_object() else {
        return Err(invalid("Parameters must be an object"));
    };
    for (key, _) in fields {
        if !names.contains(&key.as_str()) {
            return Err(invalid("Unknown parameter")
                .with_details(Value::object([("parameter", Value::string(key))])));
        }
    }
    Ok(())
}
pub(crate) fn invalid(message: &str) -> ActionError {
    ActionError::new(ErrorCode::InvalidInput, message)
}

pub(crate) struct RpcError {
    pub code: i64,
    pub message: String,
    pub data: Value,
}
impl RpcError {
    pub fn new(code: i64, message: &str) -> Self {
        Self {
            code,
            message: message.into(),
            data: Value::Null,
        }
    }
    pub fn action(error: ActionError) -> Self {
        Self {
            code: if error.code == ErrorCode::InvalidInput {
                -32602
            } else {
                -31000
            },
            message: error.message.clone(),
            data: error.to_json(),
        }
    }
}
fn error_response(id: Value, error: RpcError) -> Value {
    Value::object([
        ("jsonrpc", Value::string("2.0")),
        ("id", id),
        (
            "error",
            Value::object([
                ("code", Value::i64(error.code)),
                ("message", Value::string(error.message)),
                ("data", error.data),
            ]),
        ),
    ])
}
fn encode(value: Value) -> String {
    let mut result = match json::stringify(&value) {
        Ok(text) => text,
        Err(_) => {
            let id = value.get("id").cloned().unwrap_or(Value::Null);
            json::stringify(&error_response(id, RpcError::new(-32603, "Response exceeds encoding limits"))).unwrap_or_else(|_| String::from("{\"jsonrpc\":\"2.0\",\"id\":null,\"error\":{\"code\":-32603,\"message\":\"Encoding failed\"}}"))
        }
    };
    result.push('\n');
    result
}

/// Base64-encode a separately requested resource payload.
pub fn base64(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for part in bytes.chunks(3) {
        let a = part[0];
        let b = part.get(1).copied().unwrap_or(0);
        let c = part.get(2).copied().unwrap_or(0);
        out.push(DIGITS[(a >> 2) as usize] as char);
        out.push(DIGITS[(((a & 3) << 4) | (b >> 4)) as usize] as char);
        out.push(if part.len() > 1 {
            DIGITS[(((b & 15) << 2) | (c >> 6)) as usize] as char
        } else {
            '='
        });
        out.push(if part.len() > 2 {
            DIGITS[(c & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}
