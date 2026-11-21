// Transport-independent action contracts. New shared implementation; no corresponding module
// exists in the frozen assembly reference.
//
//   action   semantic action dispatch: Dispatcher, ActionSpec, ActionContext, ActionResult /
//            ActionError, batches, pages
//   schema   statically registered input / output schemas (a JSON Schema subset), validation
//   rpc      versioned newline JSON-RPC (Protocol, RpcSession) with MCP adapters; no I/O
//   task     bounded cooperative tasks for long operations
//
//   json, jmap, record!, wire_enum!   re-exports of formats::{json, jmap} (they lived here until
//            0.3.0; new code names formats)
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

pub mod action;
pub mod rpc;
pub mod schema;
pub mod task;
mod mcp;

pub use action::{Access, ActionContext, ActionError, ActionResult, ActionSpec, BatchCall, BatchResult,
    Dispatcher, ErrorCode, Host, Invocation, Page, Resource};
pub use json::Value;
/// the JSON of the action contracts: formats::json (here until 0.3.0)
pub use formats::json;
/// typed JSON records: formats::jmap (here until 0.3.0)
pub use formats::jmap;
pub use formats::{record, wire_enum};
pub use rpc::{Protocol, RpcSession};
