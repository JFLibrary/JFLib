// MCP 2026-07-28 stateless metadata and 2025-11-25 legacy initialization adapter.
// Implemented from the public MCP transport/schema contracts; no external SDK/code.

use crate::rpc::{
    allowed, base64, control, optional_str, required_str, RpcError, RpcSession, MCP_LEGACY_VERSION,
    MCP_VERSION,
};
use crate::{
    json,
    schema::{self, Property, Schema},
    Dispatcher, Host, Value,
};
use alloc::{string::String, vec, vec::Vec};

static NAME: Schema = Schema::String {
    min: 1,
    max: 128,
    choices: &[],
};
static QUERY: Schema = Schema::String {
    min: 0,
    max: 256,
    choices: &[],
};
static LIMIT: Schema = Schema::Unsigned { min: 1, max: 100 };
static RESOURCE_LIMIT: Schema = Schema::Unsigned {
    min: 1,
    max: 65_536,
};
static SEARCH: Schema = Schema::Object {
    properties: &[
        Property {
            name: "query",
            schema: &QUERY,
            required: false,
        },
        Property {
            name: "cursor",
            schema: &NAME,
            required: false,
        },
        Property {
            name: "limit",
            schema: &LIMIT,
            required: false,
        },
    ],
    additional: false,
};
static DESCRIBE: Schema = Schema::Object {
    properties: &[Property {
        name: "action",
        schema: &NAME,
        required: true,
    }],
    additional: false,
};
static EXECUTE: Schema = Schema::Object {
    properties: &[
        Property {
            name: "action",
            schema: &NAME,
            required: true,
        },
        Property {
            name: "input",
            schema: &schema::ANY,
            required: false,
        },
        Property {
            name: "expectedRevision",
            schema: &schema::U64,
            required: false,
        },
    ],
    additional: false,
};
static CALLS: Schema = Schema::Array {
    items: &EXECUTE,
    min: 1,
    max: 128,
};
static BATCH: Schema = Schema::Object {
    properties: &[
        Property {
            name: "calls",
            schema: &CALLS,
            required: true,
        },
        Property {
            name: "atomic",
            schema: &schema::BOOLEAN,
            required: false,
        },
        Property {
            name: "stopOnError",
            schema: &schema::BOOLEAN,
            required: false,
        },
    ],
    additional: false,
};
static TASK: Schema = Schema::Object {
    properties: &[Property {
        name: "id",
        schema: &schema::U64,
        required: true,
    }],
    additional: false,
};
static RESOURCE: Schema = Schema::Object {
    properties: &[
        Property {
            name: "uri",
            schema: &schema::STRING,
            required: true,
        },
        Property {
            name: "offset",
            schema: &schema::U64,
            required: false,
        },
        Property {
            name: "limit",
            schema: &RESOURCE_LIMIT,
            required: false,
        },
    ],
    additional: false,
};

struct Tool {
    name: &'static str,
    method: &'static str,
    description: &'static str,
    schema: &'static Schema,
    read: bool,
}

const TOOLS: &[Tool] = &[
    Tool { name: "jfgui.actions.batch", method: "actions/batch", description: "Execute ordered registered actions. Atomic batches require one supported transaction domain; otherwise rejected before execution.", schema: &BATCH, read: false },
    Tool { name: "jfgui.actions.describe", method: "actions/describe", description: "Get the exact input/output schemas, availability and revision/undo/capability contract of one action.", schema: &DESCRIBE, read: true },
    Tool { name: "jfgui.actions.execute", method: "actions/execute", description: "Execute a discovered application action. Supply expectedRevision for revision-checked mutations.", schema: &EXECUTE, read: false },
    Tool { name: "jfgui.actions.search", method: "actions/list", description: "Search registered actions by name/description. Results are paginated; fetch schemas only for the actions needed.", schema: &SEARCH, read: true },
    Tool { name: "jfgui.resources.read", method: "resources/read", description: "Read a bounded chunk of an application resource as base64, using its opaque URI and byte offset.", schema: &RESOURCE, read: true },
    Tool { name: "jfgui.state.inspect", method: "state/inspect", description: "Inspect semantic app state. Query filters, pagination and sinceRevision are defined by the application's inspection contract.", schema: &schema::OBJECT, read: true },
    Tool { name: "jfgui.tasks.cancel", method: "tasks/cancel", description: "Request cancellation of an application task at its next defined safe interruption point.", schema: &TASK, read: false },
    Tool { name: "jfgui.tasks.status", method: "tasks/get", description: "Query a task by explicit identifier for progress, completion, failure or cancellation.", schema: &TASK, read: true },
    Tool { name: "jfgui.view.render", method: "view/render", description: "Invoke the registered ui.render action and receive image resource links. Describe ui.render for its exact parameters.", schema: &schema::OBJECT, read: true },
];

impl RpcSession {
    pub(crate) fn mcp_request(
        &mut self,
        dispatcher: &Dispatcher,
        host: &mut impl Host,
        method: &str,
        params: &Value,
    ) -> Result<Value, RpcError> {
        if method == "initialize"
            && params
                .get("_meta")
                .and_then(|m| m.get("io.modelcontextprotocol/protocolVersion"))
                .is_none()
        {
            if self.legacy_initialized {
                return Err(RpcError::new(
                    -32600,
                    "Legacy initialization already completed",
                ));
            }
            if params
                .get("protocolVersion")
                .and_then(Value::as_str)
                .is_none()
                || params
                    .get("capabilities")
                    .and_then(Value::as_object)
                    .is_none()
                || !valid_info(params.get("clientInfo"))
            {
                return Err(RpcError::new(
                    -32602,
                    "Invalid legacy initialization parameters",
                ));
            }
            self.legacy_initialized = true;
            return Ok(Value::object([("protocolVersion", Value::string(MCP_LEGACY_VERSION)), ("capabilities", capabilities()), ("serverInfo", self.server_info()), ("instructions", Value::string("Search actions, describe the required schema, inspect state, then execute with expectedRevision. No embedded AI is started."))]));
        }
        let modern = if let Some(meta) = params
            .get("_meta")
            .filter(|m| m.get("io.modelcontextprotocol/protocolVersion").is_some())
        {
            let Some(version) = meta
                .get("io.modelcontextprotocol/protocolVersion")
                .and_then(Value::as_str)
            else {
                return Err(RpcError::new(
                    -32602,
                    "MCP protocol version must be a string",
                ));
            };
            if version != MCP_VERSION {
                return Err(RpcError {
                    code: -32022,
                    message: String::from("Unsupported MCP protocol version"),
                    data: Value::object([
                        ("requested", Value::string(version)),
                        ("supported", Value::Array(vec![Value::string(MCP_VERSION)])),
                    ]),
                });
            }
            if meta
                .get("io.modelcontextprotocol/clientCapabilities")
                .and_then(Value::as_object)
                .is_none()
            {
                return Err(RpcError::new(
                    -32602,
                    "Per-request clientCapabilities is required",
                ));
            }
            if meta.get("io.modelcontextprotocol/clientInfo").is_some()
                && !valid_info(meta.get("io.modelcontextprotocol/clientInfo"))
            {
                return Err(RpcError::new(-32602, "Invalid per-request clientInfo"));
            }
            true
        } else {
            if !self.legacy_initialized || (!self.legacy_ready && method != "ping") {
                return Err(RpcError::new(
                    -32600,
                    "Provide modern MCP request metadata or complete legacy initialization",
                ));
            }
            false
        };
        let mut params = params.clone();
        if let Value::Object(fields) = &mut params {
            fields.retain(|(key, _)| key != "_meta");
        }
        let mut result = match method {
            "server/discover" if modern => {
                allowed(&params, &[]).map_err(RpcError::action)?;
                Value::object([("supportedVersions", Value::Array(vec![Value::string(MCP_VERSION)])), ("capabilities", capabilities()), ("ttlMs", Value::u64(0)), ("cacheScope", Value::string("private")), ("instructions", Value::string("Search semantic actions, fetch their schemas, inspect revision, then execute. Resources and tasks use explicit application-owned identifiers."))])
            }
            "ping" => {
                allowed(&params, &[]).map_err(RpcError::action)?;
                Value::object([])
            }
            "tools/list" => {
                allowed(&params, &["cursor"]).map_err(RpcError::action)?;
                let cursor = optional_str(&params, "cursor").map_err(RpcError::action)?;
                if cursor.is_some_and(|name| !TOOLS.iter().any(|t| t.name == name)) {
                    return Err(RpcError::new(-32602, "Unknown tools cursor"));
                }
                let tools: Vec<Value> = TOOLS
                    .iter()
                    .filter(|t| cursor.is_none_or(|name| t.name > name))
                    .map(|t| {
                        Value::object([
                            ("name", Value::string(t.name)),
                            ("description", Value::string(t.description)),
                            ("inputSchema", t.schema.to_json()),
                            ("outputSchema", schema::OBJECT.to_json()),
                            (
                                "annotations",
                                Value::object([
                                    ("readOnlyHint", Value::Bool(t.read)),
                                    ("openWorldHint", Value::Bool(false)),
                                ]),
                            ),
                        ])
                    })
                    .collect();
                let mut value = Value::object([("tools", Value::Array(tools))]);
                if modern {
                    value.insert("ttlMs", Value::u64(0));
                    value.insert("cacheScope", Value::string("private"));
                }
                value
            }
            "tools/call" => {
                allowed(&params, &["name", "arguments"]).map_err(RpcError::action)?;
                let name = required_str(&params, "name").map_err(RpcError::action)?;
                let Some(tool) = TOOLS.iter().find(|t| t.name == name) else {
                    return Err(RpcError::new(-32602, "Unknown tool"));
                };
                let empty = Value::object([]);
                let arguments = params.get("arguments").unwrap_or(&empty);
                let result = tool
                    .schema
                    .validate(arguments)
                    .map_err(|e| {
                        crate::ActionError::new(
                            crate::ErrorCode::InvalidInput,
                            "Tool arguments fail their schema",
                        )
                        .with_details(Value::object([
                            ("path", Value::string(e.path)),
                            ("reason", Value::string(e.reason)),
                        ]))
                    })
                    .and_then(|()| control(dispatcher, host, tool.method, arguments));
                let is_error = result.is_err();
                let data = match result {
                    Ok(value) => Value::object([("ok", Value::Bool(true)), ("data", value)]),
                    Err(error) => {
                        Value::object([("ok", Value::Bool(false)), ("error", error.to_json())])
                    }
                };
                let text = json::stringify(&data)
                    .map_err(|_| RpcError::new(-32603, "Tool result exceeds encoding limits"))?;
                Value::object([
                    (
                        "content",
                        Value::Array(vec![Value::object([
                            ("type", Value::string("text")),
                            ("text", Value::string(text)),
                        ])]),
                    ),
                    ("structuredContent", data),
                    ("isError", Value::Bool(is_error)),
                ])
            }
            "resources/list" => {
                allowed(&params, &["cursor"]).map_err(RpcError::action)?;
                if params.get("cursor").is_some() {
                    return Err(RpcError::new(
                        -32602,
                        "Resources are discovered through action results",
                    ));
                }
                Value::object([("resources", Value::Array(Vec::new()))])
            }
            "resources/templates/list" => {
                allowed(&params, &[]).map_err(RpcError::action)?;
                Value::object([("resourceTemplates", Value::Array(Vec::new()))])
            }
            "resources/read" => {
                allowed(&params, &["uri"]).map_err(RpcError::action)?;
                let uri = required_str(&params, "uri").map_err(RpcError::action)?;
                let info = host.resource_info(uri).map_err(|_| RpcError {
                    code: -32602,
                    message: "Resource not found".into(),
                    data: Value::object([("uri", Value::string(uri))]),
                })?;
                let Some(size) = info.size.filter(|size| *size <= 65_536) else {
                    return Err(RpcError::new(
                        -32602,
                        "Use jfgui.resources.read for resources above 64 KiB or of unknown size",
                    ));
                };
                let data = host
                    .resource_read(uri, 0, size as usize)
                    .map_err(RpcError::action)?;
                if data.len() != size as usize {
                    return Err(RpcError::new(-32603, "Resource changed during read"));
                }
                Value::object([(
                    "contents",
                    Value::Array(vec![Value::object([
                        ("uri", Value::string(uri)),
                        ("mimeType", Value::string(info.mime_type)),
                        ("blob", Value::string(base64(&data))),
                    ])]),
                )])
            }
            _ => return Err(RpcError::new(-32601, "Method not found")),
        };
        if modern {
            result.insert("resultType", Value::string("complete"));
            result.insert(
                "_meta",
                Value::object([("io.modelcontextprotocol/serverInfo", self.server_info())]),
            );
        }
        Ok(result)
    }
}

fn capabilities() -> Value {
    Value::object([
        (
            "tools",
            Value::object([("listChanged", Value::Bool(false))]),
        ),
        (
            "resources",
            Value::object([
                ("subscribe", Value::Bool(false)),
                ("listChanged", Value::Bool(false)),
            ]),
        ),
    ])
}
fn valid_info(value: Option<&Value>) -> bool {
    value.is_some_and(|v| {
        v.get("name")
            .and_then(Value::as_str)
            .is_some_and(|s| !s.is_empty() && s.len() <= 256)
            && v.get("version")
                .and_then(Value::as_str)
                .is_some_and(|s| !s.is_empty() && s.len() <= 256)
    })
}
