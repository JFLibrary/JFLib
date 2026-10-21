// Native contract tests for the shared, transport-free action/JSON/MCP library.
// New test app; it has no frozen assembly counterpart and opens no external endpoint.
#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;

use agent_core::{
    json::{self, Limits},
    schema::{self, Property, Schema},
    task::{Status, TaskTable},
    ActionContext, ActionError, ActionResult, ActionSpec, BatchCall, Dispatcher, ErrorCode, Host,
    Invocation, Protocol, Resource, RpcSession, Value,
};
use alloc::{format, string::String, vec, vec::Vec};

static DELTA: Schema = Schema::Integer { min: -50, max: 50 };
static COUNT: Schema = Schema::Integer {
    min: i64::MIN,
    max: i64::MAX,
};
static INPUT: Schema = Schema::Object {
    properties: &[Property {
        name: "delta",
        schema: &DELTA,
        required: true,
    }],
    additional: false,
};
static ACTIONS: &[ActionSpec] = &[
    ActionSpec {
        transaction: Some("counter"),
        ..ActionSpec::read("counter.get", "Read the counter").schemas(&schema::EMPTY_OBJECT, &COUNT)
    },
    ActionSpec {
        undo: true,
        transaction: Some("counter"),
        command_id: Some(1),
        ..ActionSpec::write("counter.add", "Add to the counter").schemas(&INPUT, &COUNT)
    },
    ActionSpec {
        ..ActionSpec::write("counter.nonatomic", "Non-atomic mutation").schemas(&INPUT, &COUNT)
    },
    ActionSpec {
        capabilities: 1,
        ..ActionSpec::read("platform.file", "Read a platform file")
    },
    ActionSpec::read("result.invalid", "Return a deliberately broken host result"),
];

#[derive(Default)]
struct Counter {
    value: i64,
    revision: u64,
    calls: u32,
    modal: bool,
    snapshot: Option<(i64, u64)>,
    commits: u32,
}

impl Host for Counter {
    fn actions(&self) -> &[ActionSpec] {
        ACTIONS
    }
    fn revision(&self) -> u64 {
        self.revision
    }
    fn capabilities(&self) -> u64 {
        0
    }
    fn availability(&self, action: &ActionSpec) -> Result<(), ActionError> {
        if self.modal && action.mutation {
            Err(ActionError::new(
                ErrorCode::Unavailable,
                "A modal dialog is active",
            ))
        } else {
            Ok(())
        }
    }
    fn execute(
        &mut self,
        action: &ActionSpec,
        input: &Value,
        _context: &ActionContext,
    ) -> Result<ActionResult, ActionError> {
        self.calls += 1;
        if action.name == "counter.add" || action.name == "counter.nonatomic" {
            let delta = input.get("delta").and_then(Value::as_i64).unwrap_or(0);
            self.value += delta;
            self.revision += 1;
            if delta == 13 {
                return Err(ActionError::new(
                    ErrorCode::Unavailable,
                    "Simulated operation failure",
                ));
            }
        }
        Ok(ActionResult::completed(
            Value::i64(self.value),
            self.revision,
        ))
    }
    fn supports_transaction(&self, domain: &str) -> bool {
        domain == "counter"
    }
    fn begin_transaction(&mut self, _domain: &str) -> Result<(), ActionError> {
        if self.snapshot.is_some() {
            return Err(ActionError::new(
                ErrorCode::Unavailable,
                "Already in transaction",
            ));
        }
        self.snapshot = Some((self.value, self.revision));
        Ok(())
    }
    fn commit_transaction(&mut self) -> Result<(), ActionError> {
        self.snapshot = None;
        self.commits += 1;
        Ok(())
    }
    fn rollback_transaction(&mut self) -> Result<(), ActionError> {
        let Some((value, revision)) = self.snapshot.take() else {
            return Err(ActionError::new(ErrorCode::Internal, "No transaction"));
        };
        self.value = value;
        self.revision = revision;
        Ok(())
    }
    fn inspect(&self, query: &Value) -> Result<Value, ActionError> {
        let unchanged = query.get("sinceRevision").and_then(Value::as_u64) == Some(self.revision);
        Ok(Value::object([
            ("revision", Value::u64(self.revision)),
            ("unchanged", Value::Bool(unchanged)),
            (
                "value",
                if unchanged {
                    Value::Null
                } else {
                    Value::i64(self.value)
                },
            ),
        ]))
    }
    fn resource_info(&self, uri: &str) -> Result<Resource, ActionError> {
        if uri != "agent://counter/data" {
            return Err(ActionError::new(ErrorCode::NotFound, "Unknown resource"));
        }
        Ok(Resource {
            uri: uri.into(),
            name: "data".into(),
            mime_type: "application/octet-stream".into(),
            size: Some(4),
        })
    }
    fn resource_read(&self, uri: &str, offset: u64, limit: usize) -> Result<Vec<u8>, ActionError> {
        self.resource_info(uri)?;
        let bytes = b"abcd";
        let offset = offset.min(4) as usize;
        Ok(bytes[offset..offset + limit.min(4 - offset)].to_vec())
    }
}

struct Checks {
    total: usize,
    failed: usize,
}

impl Checks {
    fn check(&mut self, name: &str, good: bool) {
        self.total += 1;
        if !good {
            self.failed += 1;
            plat::write_stderr(format!("FAIL {name}\n").as_bytes());
        }
    }
}

fn input(delta: i64) -> Value {
    Value::object([("delta", Value::i64(delta))])
}
fn invoke(
    dispatcher: &Dispatcher,
    host: &mut Counter,
    name: &str,
    value: &Value,
    revision: Option<u64>,
) -> Result<ActionResult, ActionError> {
    dispatcher.execute(
        host,
        Invocation {
            action: name,
            input: value,
            expected_revision: revision,
        },
    )
}
fn code<T>(result: Result<T, ActionError>, expected: ErrorCode) -> bool {
    result.err().is_some_and(|e| e.code == expected)
}
fn batch(delta: i64, revision: u64) -> BatchCall {
    BatchCall {
        action: "counter.add".into(),
        input: input(delta),
        expected_revision: Some(revision),
    }
}

fn json_checks(checks: &mut Checks) {
    let example = br#"{"text":"Gr\u00fc\u00dfe \ud83d\ude00\n\t\u0000","number":18446744073709551615,"array":[null,true,false,-12.5e+2]}"#;
    let parsed = json::parse(example);
    checks.check(
        "json_unicode",
        parsed
            .as_ref()
            .ok()
            .and_then(|v| v.get("text"))
            .and_then(Value::as_str)
            == Some("Grüße 😀\n\t\0"),
    );
    checks.check(
        "json_exact_integer",
        parsed
            .as_ref()
            .ok()
            .and_then(|v| v.get("number"))
            .and_then(Value::as_u64)
            == Some(u64::MAX),
    );
    if let Ok(value) = parsed {
        let encoded = json::stringify(&value);
        checks.check(
            "json_roundtrip",
            encoded
                .as_ref()
                .ok()
                .and_then(|s| json::parse(s.as_bytes()).ok())
                .as_ref()
                == Some(&value),
        );
        checks.check(
            "json_one_line",
            encoded
                .ok()
                .is_some_and(|text| !text.contains('\n') && !text.contains('\r')),
        );
    }
    for invalid in [
        "",
        " ",
        "[1,]",
        "{\"a\":1,}",
        "{\"a\":1,\"a\":2}",
        "{\"a\":1,\"\\u0061\":2}",
        "[",
        "{",
        "true false",
        "NaN",
        "Infinity",
        "01",
        "-01",
        "+1",
        "1.",
        "1e",
        "1e+",
        ".5",
        "\"\\ud800\"",
        "\"\\udc00\"",
        "\"\\ud800x\"",
        "\"\\ud800\\u1234\"",
        "\"\\q\"",
        "\"a\nb\"",
        "\"unterminated",
        "null\0",
    ] {
        checks.check(
            &format!("json_reject_{invalid:?}"),
            json::parse(invalid.as_bytes()).is_err(),
        );
    }
    checks.check(
        "json_invalid_utf8",
        json::parse(&[b'"', 0xc0, 0xaf, b'"']).is_err(),
    );
    checks.check(
        "json_depth_limit",
        json::parse_with_limits(
            b"[[[0]]]",
            Limits {
                depth: 2,
                ..Limits::default()
            },
        )
        .is_err(),
    );
    checks.check(
        "json_value_limit",
        json::parse_with_limits(
            b"[0,1,2]",
            Limits {
                values: 3,
                ..Limits::default()
            },
        )
        .is_err(),
    );
    checks.check(
        "json_member_limit",
        json::parse_with_limits(
            b"[0,1]",
            Limits {
                members: 1,
                ..Limits::default()
            },
        )
        .is_err(),
    );
    checks.check(
        "json_string_limit",
        json::parse_with_limits(
            b"\"abcd\"",
            Limits {
                string_bytes: 3,
                ..Limits::default()
            },
        )
        .is_err(),
    );
    checks.check(
        "json_byte_limit",
        json::parse_with_limits(
            b"[0]",
            Limits {
                bytes: 2,
                ..Limits::default()
            },
        )
        .is_err(),
    );
    checks.check(
        "json_write_limit",
        json::stringify_with_limits(
            &Value::string("abcd"),
            Limits {
                bytes: 5,
                ..Limits::default()
            },
        )
        .is_err(),
    );
    checks.check(
        "json_write_duplicate",
        json::stringify(&Value::Object(vec![
            ("x".into(), Value::Null),
            ("x".into(), Value::Null),
        ]))
        .is_err(),
    );
    for value in [i64::MIN, -1, 0, 1, i64::MAX] {
        checks.check(
            "json_signed_extremes",
            json::parse(
                json::stringify(&Value::i64(value))
                    .unwrap_or_default()
                    .as_bytes(),
            )
            .ok()
            .and_then(|v| v.as_i64())
                == Some(value),
        );
    }
    // A deterministic parser corpus: every single byte is fed both naked and as string data.
    for byte in 0..=255u8 {
        for bytes in [vec![byte], vec![b'"', byte, b'"']] {
            if let Ok(value) = json::parse(&bytes) {
                checks.check(
                    "json_byte_corpus_roundtrip",
                    json::stringify(&value)
                        .ok()
                        .and_then(|s| json::parse(s.as_bytes()).ok())
                        == Some(value),
                );
            }
        }
    }
    checks.check("base64_empty", agent_core::rpc::base64(b"").is_empty());
    checks.check("base64_1", agent_core::rpc::base64(b"f") == "Zg==");
    checks.check("base64_2", agent_core::rpc::base64(b"fo") == "Zm8=");
    checks.check("base64_3", agent_core::rpc::base64(b"foo") == "Zm9v");
    checks.check("base64_6", agent_core::rpc::base64(b"foobar") == "Zm9vYmFy");
    for (number, expected) in [
        ("1.0", Some(1)),
        ("100e-2", Some(1)),
        ("12.30e1", Some(123)),
        ("-0e99999", Some(0)),
        ("1e-1", None),
        ("1e99999", None),
        ("18446744073709551615.0", Some(u64::MAX)),
        ("18446744073709551616", None),
    ] {
        checks.check(
            "json_exact_integral_numbers",
            json::parse(number.as_bytes()).ok().and_then(|v| v.as_u64()) == expected,
        );
    }
}

fn schema_checks(checks: &mut Checks) {
    checks.check("schema_valid", INPUT.validate(&input(12)).is_ok());
    checks.check("schema_range", INPUT.validate(&input(51)).is_err());
    checks.check(
        "schema_missing",
        INPUT
            .validate(&Value::object([]))
            .err()
            .is_some_and(|v| v.path == "/delta" && v.reason == "required"),
    );
    checks.check(
        "schema_extra",
        INPUT
            .validate(&Value::object([
                ("delta", Value::i64(1)),
                ("junk", Value::Null),
            ]))
            .is_err(),
    );
    checks.check(
        "schema_type",
        INPUT
            .validate(&Value::object([("delta", Value::string("12"))]))
            .is_err(),
    );
    checks.check(
        "schema_unsigned_full",
        schema::U64.validate(&Value::u64(u64::MAX)).is_ok(),
    );
    checks.check(
        "schema_signed_overflow",
        COUNT.validate(&Value::u64(u64::MAX)).is_err(),
    );
    let unicode = Schema::String {
        min: 2,
        max: 2,
        choices: &[],
    };
    checks.check(
        "schema_unicode_length",
        unicode.validate(&Value::string("ä😀")).is_ok(),
    );
    let choice = Schema::String {
        min: 1,
        max: 10,
        choices: &["start", "stop"],
    };
    checks.check(
        "schema_enum",
        choice.validate(&Value::string("pause")).is_err(),
    );
    let array = Schema::Array {
        items: &schema::U64,
        min: 1,
        max: 2,
    };
    checks.check(
        "schema_array_type",
        array
            .validate(&Value::Array(vec![Value::Bool(true)]))
            .is_err(),
    );
    checks.check(
        "schema_array_length",
        array.validate(&Value::Array(Vec::new())).is_err(),
    );
    let one = Schema::OneOf(&[Schema::Any, Schema::Boolean]);
    checks.check(
        "schema_oneof_ambiguity",
        one.validate(&Value::Bool(true)).is_err(),
    );
    checks.check(
        "schema_json",
        json::stringify(&INPUT.to_json())
            .ok()
            .and_then(|s| json::parse(s.as_bytes()).ok())
            .is_some(),
    );
}

fn action_checks(checks: &mut Checks) {
    let mut dispatch = Dispatcher::new();
    let mut host = Counter::default();
    let empty = Value::object([]);
    checks.check(
        "disabled_default",
        code(
            invoke(&dispatch, &mut host, "counter.get", &empty, None),
            ErrorCode::Disabled,
        ) && host.calls == 0,
    );
    checks.check(
        "gui_handler_without_endpoint",
        dispatch.execute_command(&mut host, 1, &input(2)).is_ok()
            && host.value == 2
            && !dispatch.enabled(),
    );
    dispatch.set_enabled(true);
    host = Counter::default();
    checks.check(
        "unknown_action",
        code(
            invoke(&dispatch, &mut host, "unknown", &empty, None),
            ErrorCode::NotFound,
        ),
    );
    checks.check(
        "schema_before_execution",
        code(
            invoke(&dispatch, &mut host, "counter.add", &input(51), Some(0)),
            ErrorCode::InvalidInput,
        ) && host.calls == 0,
    );
    checks.check(
        "revision_required",
        code(
            invoke(&dispatch, &mut host, "counter.add", &input(1), None),
            ErrorCode::InvalidInput,
        ) && host.calls == 0,
    );
    checks.check(
        "capability_reason",
        code(
            invoke(&dispatch, &mut host, "platform.file", &empty, None),
            ErrorCode::CapabilityUnavailable,
        ) && host.calls == 0,
    );
    host.modal = true;
    checks.check(
        "modal_block",
        code(
            invoke(&dispatch, &mut host, "counter.add", &input(1), Some(0)),
            ErrorCode::Unavailable,
        ) && host.calls == 0,
    );
    host.modal = false;
    checks.check(
        "mutation",
        invoke(&dispatch, &mut host, "counter.add", &input(2), Some(0))
            .ok()
            .is_some_and(|r| r.revision == 1 && r.value.as_i64() == Some(2)),
    );
    checks.check(
        "stale_revision",
        code(
            invoke(&dispatch, &mut host, "counter.add", &input(1), Some(0)),
            ErrorCode::RevisionConflict,
        ) && host.value == 2
            && host.calls == 1,
    );
    checks.check(
        "invalid_host_output",
        code(
            invoke(&dispatch, &mut host, "result.invalid", &empty, None),
            ErrorCode::Internal,
        ),
    );
    let page = dispatch.discover(&host, "counter", None, 1).ok();
    checks.check(
        "discovery_sorted",
        page.as_ref()
            .and_then(|p| p.actions.first())
            .and_then(|v| v.get("name"))
            .and_then(Value::as_str)
            == Some("counter.add"),
    );
    let next = page.and_then(|p| p.next_cursor);
    checks.check(
        "discovery_cursor",
        next.as_deref() == Some("counter.add")
            && dispatch
                .discover(&host, "counter", next.as_deref(), 1)
                .ok()
                .and_then(|p| p.actions.first().cloned())
                .and_then(|v| v.get("name").cloned())
                .and_then(|v| v.as_str().map(String::from))
                .as_deref()
                == Some("counter.get"),
    );
    checks.check(
        "describe_schema",
        dispatch
            .describe(&host, "counter.add")
            .ok()
            .is_some_and(|v| {
                v.get("inputSchema").is_some()
                    && v.get("revisionRequired").and_then(Value::as_bool) == Some(true)
            }),
    );
    checks.check(
        "duplicate_registry",
        agent_core::action::validate_registry(&[ACTIONS[0], ACTIONS[0]]).is_err(),
    );
    host = Counter::default();
    let unsupported = BatchCall {
        action: "counter.nonatomic".into(),
        ..batch(1, 0)
    };
    checks.check(
        "atomic_unsupported_preflight",
        code(
            dispatch.batch(&mut host, &[batch(1, 0), unsupported], true, true),
            ErrorCode::Unsupported,
        ) && host.calls == 0
            && host.value == 0,
    );
    checks.check(
        "atomic_schema_preflight",
        code(
            dispatch.batch(&mut host, &[batch(1, 0), batch(99, 0)], true, true),
            ErrorCode::InvalidInput,
        ) && host.calls == 0,
    );
    checks.check(
        "atomic_rollback",
        dispatch
            .batch(&mut host, &[batch(2, 0), batch(13, 0)], true, true)
            .ok()
            .is_some_and(|r| r.rolled_back && r.results.len() == 2)
            && host.value == 0
            && host.revision == 0,
    );
    checks.check(
        "atomic_commit",
        dispatch
            .batch(&mut host, &[batch(2, 0), batch(3, 0)], true, true)
            .ok()
            .is_some_and(|r| !r.rolled_back && r.results.len() == 2)
            && host.value == 5
            && host.commits == 1,
    );
    host = Counter::default();
    checks.check(
        "ordered_partial_error",
        dispatch
            .batch(
                &mut host,
                &[batch(1, 0), batch(2, 0), batch(3, 1)],
                false,
                false,
            )
            .ok()
            .is_some_and(|r| r.results.len() == 3 && r.results[1].is_err() && r.results[2].is_ok())
            && host.value == 4,
    );
    let mut other = Counter::default();
    checks.check(
        "instance_isolation",
        invoke(&dispatch, &mut other, "counter.get", &empty, None)
            .ok()
            .and_then(|r| r.value.as_i64())
            == Some(0),
    );
    dispatch.set_enabled(false);
    checks.check(
        "disabled_after_use",
        code(dispatch.discover(&host, "", None, 10), ErrorCode::Disabled),
    );
}

fn task_checks(checks: &mut Checks) {
    let mut table = TaskTable::new(2);
    let first = table.create("counter.background", true).unwrap_or(0);
    let second = table.create("counter.save", false).unwrap_or(0);
    checks.check(
        "tasks_limit",
        code(
            table.create("counter.third", true),
            ErrorCode::LimitExceeded,
        ),
    );
    checks.check("tasks_start", table.checkpoint(first).ok() == Some(true));
    checks.check(
        "tasks_progress",
        table.update(first, Some(250), "Quarter done").is_ok(),
    );
    checks.check(
        "tasks_bad_progress",
        code(table.update(first, Some(1001), ""), ErrorCode::InvalidInput),
    );
    checks.check(
        "tasks_cancel_request",
        table
            .request_cancel(first)
            .ok()
            .is_some_and(|t| t.status == Status::Cancelling),
    );
    checks.check(
        "tasks_not_completed_early",
        code(
            table.finish(first, ActionResult::completed(Value::Null, 0)),
            ErrorCode::Unavailable,
        ),
    );
    checks.check(
        "tasks_cancel_checkpoint",
        table.checkpoint(first).ok() == Some(false)
            && table
                .get(first)
                .ok()
                .is_some_and(|t| t.status == Status::Cancelled),
    );
    checks.check(
        "tasks_uncancellable",
        code(table.request_cancel(second), ErrorCode::Unsupported),
    );
    checks.check(
        "tasks_no_forget_running",
        code(table.forget(second), ErrorCode::Unavailable),
    );
    checks.check(
        "tasks_finish",
        table
            .finish(second, ActionResult::completed(Value::Null, 0))
            .is_ok(),
    );
    checks.check(
        "tasks_pagination",
        table
            .list(0, 1)
            .ok()
            .and_then(|v| v.get("nextCursor").and_then(Value::as_u64))
            == Some(first),
    );
    checks.check("tasks_forget", table.forget(first).is_ok());
    checks.check(
        "tasks_ids_not_reused",
        table
            .create("counter.next", true)
            .ok()
            .is_some_and(|id| id > second),
    );
    checks.check(
        "tasks_other_instance",
        code(TaskTable::default().get(second), ErrorCode::NotFound),
    );
}

fn framing_checks(checks: &mut Checks) {
    let mut framer = agent_io::Framer::default();
    let mut records = Vec::new();
    framer.feed(b"{\"text\":\"Gr\xc3", &mut |record| records.push(record));
    checks.check("framer_partial_utf8", records.is_empty());
    framer.feed(b"\xbc\xc3\x9fe\"}\r\n{}\n", &mut |record| {
        records.push(record)
    });
    checks.check("framer_crlf_and_multiple", records.len() == 2 && matches!(&records[0], agent_io::Record::Line(line) if line == "{\"text\":\"Grüße\"}".as_bytes()) && matches!(&records[1], agent_io::Record::Line(line) if line == b"{}"));
    records.clear();
    framer.feed(b"null", &mut |record| records.push(record));
    framer.finish(&mut |record| records.push(record));
    checks.check(
        "framer_eof_record",
        records.len() == 1
            && matches!(&records[0], agent_io::Record::Line(line) if line == b"null"),
    );
    records.clear();
    framer.feed(&vec![b'x'; agent_io::LINE_LIMIT + 1], &mut |record| {
        records.push(record)
    });
    checks.check("framer_oversize_waits_for_end", records.is_empty());
    framer.feed(b"ignored\n{}\n", &mut |record| records.push(record));
    checks.check(
        "framer_oversize_recovery",
        records.len() == 2
            && matches!(&records[0], agent_io::Record::TooLong)
            && matches!(&records[1], agent_io::Record::Line(line) if line == b"{}"),
    );
    records.clear();
    framer.feed(&vec![b'x'; agent_io::LINE_LIMIT], &mut |record| {
        records.push(record)
    });
    framer.feed(b"\n", &mut |record| records.push(record));
    checks.check("framer_exact_limit", records.len() == 1 && matches!(&records[0], agent_io::Record::Line(line) if line.len() == agent_io::LINE_LIMIT));
    records.clear();
    framer.feed(&vec![b'x'; agent_io::LINE_LIMIT + 1], &mut |record| {
        records.push(record)
    });
    framer.finish(&mut |record| records.push(record));
    checks.check(
        "framer_oversize_eof",
        records.len() == 1 && matches!(&records[0], agent_io::Record::TooLong),
    );
}

fn request(method: &str, params: Value) -> Value {
    Value::object([
        ("jsonrpc", Value::string("2.0")),
        ("id", Value::u64(7)),
        ("method", Value::string(method)),
        ("params", params),
    ])
}
fn rpc(
    session: &mut RpcSession,
    dispatcher: &Dispatcher,
    host: &mut Counter,
    request: Value,
) -> Value {
    json::stringify(&request)
        .ok()
        .and_then(|line| session.handle(dispatcher, host, line.as_bytes()))
        .and_then(|line| json::parse(line.as_bytes()).ok())
        .unwrap_or(Value::Null)
}
fn modern(mut params: Value, version: &str) -> Value {
    params.insert(
        "_meta",
        Value::object([
            (
                "io.modelcontextprotocol/protocolVersion",
                Value::string(version),
            ),
            (
                "io.modelcontextprotocol/clientCapabilities",
                Value::object([]),
            ),
        ]),
    );
    params
}
fn rpc_checks(checks: &mut Checks) {
    let mut dispatcher = Dispatcher::new();
    let mut host = Counter::default();
    let mut session = RpcSession::new(Protocol::Json, "agenttest", "1");
    checks.check(
        "rpc_json_session_stateless",
        !session.requires_persistent_session(),
    );
    for malformed in [
        br#"{"jsonrpc":"2.0","id":true,"method":"agent/info"}"#.as_slice(),
        br#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":[]}"#.as_slice(),
        br#"{"jsonrpc":"2.0","id":1,"method":"agent/info","result":{}}"#.as_slice(),
    ] {
        checks.check(
            "rpc_malformed_does_not_select_protocol",
            agent_core::rpc::requested_protocol(malformed).is_none(),
        );
    }
    checks.check(
        "rpc_disabled",
        rpc(
            &mut session,
            &dispatcher,
            &mut host,
            request("agent/info", Value::object([])),
        )
        .get("error")
        .is_some(),
    );
    dispatcher.set_enabled(true);
    checks.check(
        "rpc_bad_json",
        session
            .handle(&dispatcher, &mut host, b"{")
            .and_then(|s| json::parse(s.as_bytes()).ok())
            .and_then(|v| v.get("error").cloned())
            .and_then(|v| v.get("code").and_then(Value::as_i64))
            == Some(-32700),
    );
    checks.check(
        "rpc_array_rejected",
        session
            .handle(&dispatcher, &mut host, b"[]")
            .is_some_and(|s| s.contains("-32600")),
    );
    checks.check(
        "rpc_no_response_as_request",
        session
            .handle(
                &dispatcher,
                &mut host,
                br#"{"jsonrpc":"2.0","id":1,"result":{}}"#,
            )
            .is_some_and(|s| s.contains("-32600")),
    );
    let notification = Value::object([
        ("jsonrpc", Value::string("2.0")),
        ("method", Value::string("actions/execute")),
        (
            "params",
            Value::object([
                ("action", Value::string("counter.add")),
                ("input", input(1)),
                ("expectedRevision", Value::u64(0)),
            ]),
        ),
    ]);
    checks.check(
        "rpc_notifications_no_mutation",
        json::stringify(&notification)
            .ok()
            .and_then(|s| session.handle(&dispatcher, &mut host, s.as_bytes()))
            .is_none()
            && host.calls == 0,
    );
    checks.check(
        "rpc_info_version",
        rpc(
            &mut session,
            &dispatcher,
            &mut host,
            request("agent/info", Value::object([])),
        )
        .get("result")
        .and_then(|r| r.get("protocolVersion"))
        .and_then(Value::as_str)
            == Some("1"),
    );
    checks.check(
        "rpc_version_rejection",
        rpc(
            &mut session,
            &dispatcher,
            &mut host,
            request(
                "agent/info",
                Value::object([("protocolVersion", Value::string("99"))]),
            ),
        )
        .get("error")
        .is_some(),
    );
    checks.check(
        "rpc_mutation",
        rpc(
            &mut session,
            &dispatcher,
            &mut host,
            request(
                "actions/execute",
                Value::object([
                    ("action", Value::string("counter.add")),
                    ("input", input(4)),
                    ("expectedRevision", Value::u64(0)),
                ]),
            ),
        )
        .get("result")
        .is_some()
            && host.value == 4,
    );
    checks.check(
        "rpc_resource_chunk",
        rpc(
            &mut session,
            &dispatcher,
            &mut host,
            request(
                "resources/read",
                Value::object([
                    ("uri", Value::string("agent://counter/data")),
                    ("offset", Value::u64(1)),
                    ("limit", Value::u64(2)),
                ]),
            ),
        )
        .get("result")
        .and_then(|v| v.get("data"))
        .and_then(Value::as_str)
            == Some("YmM="),
    );
    let mut mcp = RpcSession::new(Protocol::Mcp, "agenttest", "1");
    checks.check(
        "mcp_requires_explicit_era",
        rpc(
            &mut mcp,
            &dispatcher,
            &mut host,
            request("tools/list", Value::object([])),
        )
        .get("error")
        .is_some(),
    );
    let discovery = rpc(
        &mut mcp,
        &dispatcher,
        &mut host,
        request("server/discover", modern(Value::object([]), "2026-07-28")),
    );
    checks.check(
        "mcp_modern_discovery",
        discovery.get("result").is_some_and(|v| {
            v.get("resultType").and_then(Value::as_str) == Some("complete")
                && v.get("supportedVersions").is_some()
                && v.get("ttlMs").is_some()
        }),
    );
    let tools = rpc(
        &mut mcp,
        &dispatcher,
        &mut host,
        request("tools/list", modern(Value::object([]), "2026-07-28")),
    );
    checks.check(
        "mcp_small_catalog",
        tools
            .get("result")
            .and_then(|v| v.get("tools"))
            .and_then(Value::as_array)
            .is_some_and(|v| v.len() == 9),
    );
    checks.check(
        "mcp_unsupported_version",
        rpc(
            &mut mcp,
            &dispatcher,
            &mut host,
            request("tools/list", modern(Value::object([]), "2027-01-01")),
        )
        .get("error")
        .and_then(|v| v.get("code"))
        .and_then(Value::as_i64)
            == Some(-32022),
    );
    checks.check(
        "mcp_modern_metadata_not_cached",
        rpc(
            &mut mcp,
            &dispatcher,
            &mut host,
            request("tools/list", Value::object([])),
        )
        .get("error")
        .is_some(),
    );
    let call = Value::object([
        ("name", Value::string("jfgui.actions.execute")),
        (
            "arguments",
            Value::object([
                ("action", Value::string("counter.add")),
                ("input", input(2)),
                ("expectedRevision", Value::u64(0)),
            ]),
        ),
    ]);
    checks.check(
        "mcp_action_errors_are_tool_results",
        rpc(
            &mut mcp,
            &dispatcher,
            &mut host,
            request("tools/call", modern(call, "2026-07-28")),
        )
        .get("result")
        .and_then(|v| v.get("isError"))
        .and_then(Value::as_bool)
            == Some(true),
    );
    let call = Value::object([
        ("name", Value::string("jfgui.actions.execute")),
        (
            "arguments",
            Value::object([
                ("action", Value::string("counter.add")),
                ("input", input(2)),
                ("expectedRevision", Value::u64(1)),
            ]),
        ),
    ]);
    checks.check(
        "mcp_shared_dispatch",
        rpc(
            &mut mcp,
            &dispatcher,
            &mut host,
            request("tools/call", modern(call, "2026-07-28")),
        )
        .get("result")
        .and_then(|v| v.get("isError"))
        .and_then(Value::as_bool)
            == Some(false)
            && host.value == 6,
    );
    checks.check(
        "mcp_resource_read",
        rpc(
            &mut mcp,
            &dispatcher,
            &mut host,
            request(
                "resources/read",
                modern(
                    Value::object([("uri", Value::string("agent://counter/data"))]),
                    "2026-07-28",
                ),
            ),
        )
        .get("result")
        .and_then(|v| v.get("contents"))
        .and_then(Value::as_array)
        .is_some_and(|a| a.len() == 1),
    );
    let mut legacy = RpcSession::new(Protocol::Mcp, "agenttest", "1");
    let initialize = Value::object([
        ("protocolVersion", Value::string("2025-11-25")),
        ("capabilities", Value::object([])),
        (
            "clientInfo",
            Value::object([
                ("name", Value::string("test")),
                ("version", Value::string("1")),
            ]),
        ),
    ]);
    checks.check(
        "mcp_legacy_init",
        rpc(
            &mut legacy,
            &dispatcher,
            &mut host,
            request("initialize", initialize),
        )
        .get("result")
        .and_then(|v| v.get("protocolVersion"))
        .and_then(Value::as_str)
            == Some("2025-11-25"),
    );
    checks.check(
        "mcp_legacy_wait_initialized",
        rpc(
            &mut legacy,
            &dispatcher,
            &mut host,
            request("tools/list", Value::object([])),
        )
        .get("error")
        .is_some(),
    );
    checks.check(
        "mcp_legacy_session_persistent",
        legacy.requires_persistent_session(),
    );
    checks.check(
        "mcp_initialized_notification",
        legacy
            .handle(
                &dispatcher,
                &mut host,
                br#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
            )
            .is_none(),
    );
    let result = rpc(
        &mut legacy,
        &dispatcher,
        &mut host,
        request("tools/list", Value::object([])),
    );
    checks.check(
        "mcp_legacy_tools",
        result
            .get("result")
            .is_some_and(|v| v.get("tools").is_some() && v.get("resultType").is_none()),
    );
    checks.check(
        "mcp_null_id",
        legacy
            .handle(
                &dispatcher,
                &mut host,
                br#"{"jsonrpc":"2.0","id":null,"method":"ping"}"#,
            )
            .is_some_and(|s| s.contains("-32600")),
    );
}

fn runtime_lifecycle_checks(checks: &mut Checks) {
    static PROBE_ACTIONS: &[ActionSpec] = &[ActionSpec::write(
        "probe.change",
        "Change the lifecycle probe",
    )];
    struct Probe {
        changes: ui::Shared<i32>,
        exits: ui::Shared<i32>,
    }
    impl ui::App for Probe {
        fn frame(&mut self, _ui: &mut ui::Ui) {}
        fn actions(&self) -> &'static [ActionSpec] {
            PROBE_ACTIONS
        }
        fn action_revision(&self) -> Option<u64> {
            Some(self.changes.get() as u64)
        }
        fn action(
            &mut self,
            _ui: &mut ui::Ui,
            _spec: &ActionSpec,
            _input: &Value,
            _context: &ActionContext,
        ) -> Result<ActionResult, ActionError> {
            self.changes.set(self.changes.get() + 1);
            Ok(ActionResult::completed(
                Value::object([]),
                self.changes.get() as u64,
            ))
        }
        fn command(&mut self, _ui: &mut ui::Ui, id: u32, _arg: u32) -> bool {
            if id == ui::UIC_EXIT {
                self.exits.set(self.exits.get() + 1);
                return true;
            }
            false
        }
    }
    let changes = ui::Shared::new(0);
    let exits = ui::Shared::new(0);
    let mut runtime = ui::Runtime::new(
        &ui::AppDesc {
            title: "Lifecycle probe",
            w: 64,
            h: 64,
            ..ui::AppDesc::NONE
        },
        Probe {
            changes: changes.clone(),
            exits: exits.clone(),
        },
        &style::STYLE,
    )
    .unwrap();
    let mut session = RpcSession::new(Protocol::Json, "probe", "1");
    runtime.set_agent_enabled(true);
    let request = br#"{"jsonrpc":"2.0","id":1,"method":"actions/execute","params":{"action":"probe.change","expectedRevision":0}}"#;
    checks.check(
        "runtime_control_active",
        runtime
            .control(&mut session, request)
            .is_some_and(|value| !value.contains("\"error\""))
            && changes.get() == 1,
    );
    runtime.finish();
    checks.check(
        "runtime_finish_revokes_control",
        !runtime.ui().agent_enabled(),
    );
    checks.check("runtime_exit_once", exits.get() == 1);
    runtime.set_agent_enabled(true);
    checks.check(
        "runtime_cannot_reactivate_after_exit",
        !runtime.ui().agent_enabled(),
    );
    checks.check(
        "runtime_control_rejected_after_exit",
        runtime
            .control(&mut session, request)
            .is_some_and(|value| value.contains("disabled"))
            && changes.get() == 1,
    );
    checks.check("runtime_step_closed", !runtime.step(1, &[]));
    drop(runtime);
    checks.check("runtime_drop_keeps_single_exit", exits.get() == 1);
}

fn main() -> i32 {
    let args = plat::args();
    if args.iter().any(|arg| arg == "--ipc-server") {
        let mut dispatcher = Dispatcher::new();
        dispatcher.set_enabled(true);
        let mut host = Counter::default();
        let protocol = if args.iter().any(|arg| arg == "--ipc-mcp") {
            Protocol::Mcp
        } else {
            Protocol::Json
        };
        let display_name = if args.iter().any(|arg| arg == "--long-title") {
            "Unbenannt – große Grafik / Paint! mit einem langen Namen"
        } else {
            "agenttest"
        };
        let mut server = match agent_io::ipc::Server::bind(display_name, "1", protocol) {
            Ok(server) => server,
            Err(code) => {
                plat::write_stderr(format!("IPC bind failed: {code}\n").as_bytes());
                return 1;
            }
        };
        let info = Value::object([("endpoint", Value::string(server.endpoint()))]);
        plat::write_stdout(format!("{}\n", json::stringify(&info).unwrap_or_default()).as_bytes());
        let started = plat::time_us();
        while plat::time_us().saturating_sub(started) < 20_000_000 {
            server.poll(&dispatcher, &mut host);
            plat::sleep_ms(2);
        }
        return 0;
    }
    if args
        .iter()
        .any(|arg| arg == "--agent-json" || arg == "--agent-mcp")
    {
        let mut dispatcher = Dispatcher::new();
        dispatcher.set_enabled(true);
        let mut host = Counter::default();
        let protocol = if args.iter().any(|arg| arg == "--agent-mcp") {
            Protocol::Mcp
        } else {
            Protocol::Json
        };
        let mut session = RpcSession::new(protocol, "agenttest", "1");
        return agent_io::serve_stdio(|line| session.handle(&dispatcher, &mut host, line));
    }
    let mut checks = Checks {
        total: 0,
        failed: 0,
    };
    json_checks(&mut checks);
    schema_checks(&mut checks);
    action_checks(&mut checks);
    task_checks(&mut checks);
    framing_checks(&mut checks);
    rpc_checks(&mut checks);
    runtime_lifecycle_checks(&mut checks);
    plat::write_stdout(
        format!(
            "agenttest: {} checks, {} failures\n",
            checks.total, checks.failed
        )
        .as_bytes(),
    );
    if checks.failed == 0 {
        0
    } else {
        1
    }
}

ui::main!(main());
