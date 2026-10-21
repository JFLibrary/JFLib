// jmapt -- checks of agent_core::jmap (record! / wire_enum!, Field for the standard types)
// and json::stringify_pretty

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

use agent_core::jmap::{self, Field, Json, Value};
use agent_core::json::{self, Limits};
use agent_core::{record, wire_enum};

use crate::T;

wire_enum! {
    pub enum Color {
        Red = "red",
        Green = "green",
    }
}

record! {
    pub struct Inner {
        pub n: u16,
        pub on: bool,
    }
}

record! {
    pub struct Outer {
        pub name: String,
        pub big: u64,
        pub neg: i32,
        pub color: Color,
        pub items: Vec<Inner>,
        pub maybe: Option<u32>,
        pub map: BTreeMap<String, u8>,
        pub raw: Value,
    }
}

pub fn run(t: &mut T) {
    let mut map = BTreeMap::new();
    map.insert(String::from("b"), 2);
    map.insert(String::from("a"), 1);
    let o = Outer {
        name: String::from("x\"y"),
        big: u64::MAX,
        neg: -5,
        color: Color::Green,
        items: alloc::vec![Inner { n: 7, on: true }, Inner::default()],
        maybe: None,
        map,
        raw: Value::object([("k", Value::Array(Vec::new()))]),
    };
    let v = o.to_json();
    t.eq("round trip", Outer::from_json(&v), Some(o.clone()));
    let text = json::stringify(&v).unwrap_or_default();
    t.eq("compact text", text.as_str(), "{\"name\":\"x\\\"y\",\"big\":18446744073709551615,\"neg\":-5,\"color\":\"green\",\"items\":[{\"n\":7,\"on\":true},{\"n\":0,\"on\":false}],\"maybe\":null,\"map\":{\"a\":1,\"b\":2},\"raw\":{\"k\":[]}}");
    // missing and wrong members take defaults, unknown ones are ignored
    let p = json::parse(b"{\"name\": 5, \"color\": \"blue\", \"neg\": 99999999999, \"items\": [{\"n\": 70000}], \"extra\": 1}").unwrap_or(Value::Null);
    let r = Outer::from_json(&p);
    t.check("defaults", r.as_ref().is_some_and(|r| r.name.is_empty() && r.color == Color::Red && r.neg == i32::MAX && r.items.len() == 1 && r.items[0].n == u16::MAX && r.raw == Value::Null));
    t.eq("not an object", Outer::from_json(&Value::Array(Vec::new())), None);
    t.eq("option some", <Option<u32> as Field>::take(Some(&Value::u64(3))), Some(3));
    t.eq("enum names", (Color::ALL.len(), Color::parse("red"), Color::Green.as_str()), (2, Some(Color::Red), "green"));
    t.eq("list helpers", jmap::from_list::<Inner>(&jmap::list(&[Inner { n: 1, on: false }])), alloc::vec![Inner { n: 1, on: false }]);
    // pretty printing
    let small = Value::object([("a", Value::Array(alloc::vec![Value::u64(1), Value::Bool(true)])), ("e", Value::Object(Vec::new())), ("z", Value::Array(Vec::new()))]);
    t.eq("pretty", json::stringify_pretty(&small, Limits::default(), 2), Ok(String::from("{\n  \"a\": [\n    1,\n    true\n  ],\n  \"e\": {},\n  \"z\": []\n}")));
    t.eq("pretty parses back", json::stringify_pretty(&v, Limits::default(), 4).ok().and_then(|s| json::parse(s.as_bytes()).ok()), Some(v.clone()));
    let tight = Limits { bytes: 20, ..Limits::default() };
    t.check("pretty keeps the byte limit", json::stringify_pretty(&small, tight, 2).is_err_and(|e| e.kind == "byte_limit"));
}
