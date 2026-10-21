// Transport-independent action contracts and bounded JSON. New shared implementation;
// no corresponding module exists in the frozen assembly reference.
//
//   json   strict bounded JSON: Value, parse, stringify, stringify_pretty
//   jmap   typed mapping of records and string enums (Field, Json, record!, wire_enum!)
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

pub mod action;
pub mod jmap;
pub mod json;
pub mod rpc;
pub mod schema;
pub mod task;
mod mcp;

pub use action::{Access, ActionContext, ActionError, ActionResult, ActionSpec, BatchCall, BatchResult,
    Dispatcher, ErrorCode, Host, Invocation, Page, Resource};
pub use json::Value;
pub use rpc::{Protocol, RpcSession};
