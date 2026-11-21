// jmapt -- checks of formats::jmap through agent_core's re-export (record! / wire_enum!, Field
// for the standard types), json::stringify_pretty, and that the old paths of agent_core and base
// are formats' modules

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
    // agent_core::{json, jmap} and base::{xml, encoding} are re-exports of formats, not copies:
    // a value of one path is a value of the other (this would not compile otherwise)
    let new: formats::json::Value = v.clone();
    let old: agent_core::json::Value = formats::json::parse(formats::json::stringify(&new).unwrap_or_default().as_bytes()).unwrap_or(Value::Null);
    t.eq("old json path", old, v);
    let node: Result<formats::xml::Node, base::xml::Error> = base::xml::parse("<a xmlns='urn:x' b='1'/>");
    t.check("old xml path", node.is_ok_and(|n| n.is("urn:x", "a") && n.attr("", "b") == Some("1")));
    t.eq("old encoding path", base::encoding::decode(b"\xef\xbb\xbfx", 8), Ok((String::from("x"), formats::encoding::Encoding::Utf8Bom)));
    let f: fn(&Inner) -> formats::json::Value = <Inner as formats::jmap::Json>::to_json;
    t.eq("record! of agent_core implements formats::jmap", f(&Inner { n: 2, on: true }), Inner { n: 2, on: true }.to_json());
}
