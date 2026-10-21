// util -- small helpers of the server: clock, ids, JSON answers and errors, months for the
// journals

use alloc::string::String;

use bc_proto::api::{code, ApiError};
use bc_proto::j::{Json, Value};
use netio::http::Response;

/// seconds since the epoch
pub fn now() -> i64 {
    plat::unix_time_ns().div_euclid(1_000_000_000)
}

/// a new random id (UUID v4 text)
pub fn new_id() -> String {
    bc_proto::ids::uuid(plat::random_bytes())
}

/// yyyymm of a time (UTC) for the journals
pub fn month(t: i64) -> u32 {
    let c = datetime::Zone::utc().to_local(datetime::Timestamp::from_secs(t));
    c.year as u32 * 100 + c.month as u32
}

pub fn json(status: u16, v: &Value) -> Response {
    Response::json(status, v)
}

pub fn ok<T: Json>(status: u16, v: &T) -> Response {
    Response::json(status, &v.to_json())
}

pub fn list<T: Json>(items: &[T]) -> Response {
    Response::json(200, &bc_proto::j::list(items))
}

pub fn no_content() -> Response {
    Response::new(204)
}

/// an error answer {code, message}
pub fn err(status: u16, c: &str, message: &str) -> Response {
    ok(status, &ApiError { code: String::from(c), message: String::from(message), field: String::new() })
}

/// an error about one field; the status follows the code (unauthorized 401, forbidden 403,
/// conflict 409, otherwise 400)
pub fn field(c: &str, message: &str, field: &str) -> Response {
    let status = match c {
        code::UNAUTHORIZED => 401,
        code::FORBIDDEN => 403,
        code::CONFLICT | code::IMMUTABLE => 409,
        _ => 400,
    };
    ok(status, &ApiError { code: String::from(c), message: String::from(message), field: String::from(field) })
}

pub fn bad(message: &str) -> Response {
    err(400, code::BAD_REQUEST, message)
}

pub fn not_found(what: &str) -> Response {
    err(404, code::NOT_FOUND, what)
}

pub fn conflict(message: &str) -> Response {
    err(409, code::CONFLICT, message)
}

pub fn internal(message: &str) -> Response {
    err(500, code::INTERNAL, message)
}

/// 202 {operation_id}
pub fn accepted(op: &str) -> Response {
    ok(202, &bc_proto::api::Accepted { operation_id: String::from(op) })
}

/// a record from the store
pub fn rec<T: Json>(v: Option<Value>) -> Option<T> {
    v.as_ref().and_then(T::from_json)
}

/// all records of a collection
pub fn all<T: Json>(db: &datastore::Store, coll: &str) -> alloc::vec::Vec<T> {
    db.list(coll).iter().filter_map(|(_, v)| T::from_json(v)).collect()
}

/// a JSON object with one more member (a record plus server-only fields)
pub fn with(mut v: Value, k: &str, x: Value) -> Value {
    v.insert(k, x);
    v
}
