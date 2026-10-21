// Statically registered input/output schemas and runtime validation. New shared code.

use crate::json::Value;
use alloc::{format, string::String, vec::Vec};

/// Supported JSON Schema subset, represented without parsing schema text at runtime.
#[derive(Clone, Copy, Debug)]
pub enum Schema {
    /// Any bounded JSON value.
    Any,
    /// Null only.
    Null,
    /// Boolean only.
    Boolean,
    /// Any syntactically valid JSON number.
    Number,
    /// Signed integer with inclusive bounds.
    Integer {
        /// Inclusive lower bound.
        min: i64,
        /// Inclusive upper bound.
        max: i64,
    },
    /// Unsigned integer with inclusive bounds, retaining all 64 bits.
    Unsigned {
        /// Inclusive lower bound.
        min: u64,
        /// Inclusive upper bound.
        max: u64,
    },
    /// Unicode string, with lengths in Unicode scalar values and optional allowed values.
    String {
        /// Minimum character count.
        min: usize,
        /// Maximum character count.
        max: usize,
        /// Empty accepts any string; otherwise an exact enumeration.
        choices: &'static [&'static str],
    },
    /// Array whose items all follow one schema.
    Array {
        /// Item schema.
        items: &'static Schema,
        /// Minimum number of items.
        min: usize,
        /// Maximum number of items.
        max: usize,
    },
    /// Object with named properties and an explicit policy for unknown names.
    Object {
        /// Known property definitions.
        properties: &'static [Property],
        /// Whether unknown properties are accepted.
        additional: bool,
    },
    /// Exactly one member schema must match.
    OneOf(&'static [Schema]),
}

/// Named member of an object schema.
#[derive(Clone, Copy, Debug)]
pub struct Property {
    /// Stable JSON member name.
    pub name: &'static str,
    /// Member validation contract.
    pub schema: &'static Schema,
    /// Whether omission is an error.
    pub required: bool,
}

/// Empty object accepted by parameterless actions.
pub static EMPTY_OBJECT: Schema = Schema::Object {
    properties: &[],
    additional: false,
};
/// General object, for extensible host results.
pub static OBJECT: Schema = Schema::Object {
    properties: &[],
    additional: true,
};
/// Unconstrained bounded value.
pub static ANY: Schema = Schema::Any;
/// Boolean value.
pub static BOOLEAN: Schema = Schema::Boolean;
/// A full-width unsigned integer.
pub static U64: Schema = Schema::Unsigned {
    min: 0,
    max: u64::MAX,
};
/// Typical short string.
pub static STRING: Schema = Schema::String {
    min: 0,
    max: 4096,
    choices: &[],
};

/// Validation error with a JSON Pointer to the invalid field.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Violation {
    /// JSON Pointer; empty means the root value.
    pub path: String,
    /// Stable error category suitable for automated correction.
    pub reason: &'static str,
}

impl Schema {
    /// Validate a value; unknown properties and missing required members are checked.
    pub fn validate(&self, value: &Value) -> Result<(), Violation> {
        self.validate_at(value, String::new(), 0)
    }

    fn validate_at(&self, value: &Value, path: String, depth: usize) -> Result<(), Violation> {
        let bad = |reason| {
            Err(Violation {
                path: path.clone(),
                reason,
            })
        };
        if depth > 32 {
            return bad("schema_depth_limit");
        }
        match self {
            Self::Any => Ok(()),
            Self::Null if matches!(value, Value::Null) => Ok(()),
            Self::Boolean if matches!(value, Value::Bool(_)) => Ok(()),
            Self::Number if matches!(value, Value::Number(_)) => Ok(()),
            Self::Integer { min, max } => match value.as_i64() {
                Some(v) if v >= *min && v <= *max => Ok(()),
                Some(_) => bad("out_of_range"),
                _ => bad("expected_integer"),
            },
            Self::Unsigned { min, max } => match value.as_u64() {
                Some(v) if v >= *min && v <= *max => Ok(()),
                Some(_) => bad("out_of_range"),
                _ => bad("expected_unsigned_integer"),
            },
            Self::String { min, max, choices } => {
                let Some(text) = value.as_str() else {
                    return bad("expected_string");
                };
                let count = text.chars().count();
                if count < *min || count > *max {
                    return bad("string_length");
                }
                if !choices.is_empty() && !choices.contains(&text) {
                    return bad("unknown_choice");
                }
                Ok(())
            }
            Self::Array { items, min, max } => {
                let Some(values) = value.as_array() else {
                    return bad("expected_array");
                };
                if values.len() < *min || values.len() > *max {
                    return bad("array_length");
                }
                for (index, item) in values.iter().enumerate() {
                    items.validate_at(item, format!("{path}/{index}"), depth + 1)?;
                }
                Ok(())
            }
            Self::Object {
                properties,
                additional,
            } => {
                let Some(fields) = value.as_object() else {
                    return bad("expected_object");
                };
                for property in *properties {
                    match value.get(property.name) {
                        Some(v) => property.schema.validate_at(
                            v,
                            pointer(&path, property.name),
                            depth + 1,
                        )?,
                        None if property.required => {
                            return Err(Violation {
                                path: pointer(&path, property.name),
                                reason: "required",
                            })
                        }
                        None => (),
                    }
                }
                for (index, (name, _)) in fields.iter().enumerate() {
                    if fields[..index].iter().any(|(previous, _)| previous == name) {
                        return Err(Violation {
                            path: pointer(&path, name),
                            reason: "duplicate_key",
                        });
                    }
                    if !*additional && !properties.iter().any(|p| p.name == name) {
                        return Err(Violation {
                            path: pointer(&path, name),
                            reason: "unknown_property",
                        });
                    }
                }
                Ok(())
            }
            Self::OneOf(options) => {
                let mut matches = 0;
                for option in *options {
                    if option.validate_at(value, path.clone(), depth + 1).is_ok() {
                        matches += 1;
                    }
                }
                if matches == 1 {
                    Ok(())
                } else {
                    bad("one_of")
                }
            }
            _ => bad("type_mismatch"),
        }
    }

    /// Emit a JSON Schema 2020-12 compatible representation for discovery.
    pub fn to_json(&self) -> Value {
        self.to_json_at(0)
    }

    fn to_json_at(&self, depth: usize) -> Value {
        if depth > 32 {
            return Value::Bool(false);
        }
        let typed = |name: &str| Value::object([("type", Value::string(name))]);
        match self {
            Self::Any => Value::object([]),
            Self::Null => typed("null"),
            Self::Boolean => typed("boolean"),
            Self::Number => typed("number"),
            Self::Integer { min, max } => Value::object([
                ("type", Value::string("integer")),
                ("minimum", Value::i64(*min)),
                ("maximum", Value::i64(*max)),
            ]),
            Self::Unsigned { min, max } => Value::object([
                ("type", Value::string("integer")),
                ("minimum", Value::u64(*min)),
                ("maximum", Value::u64(*max)),
            ]),
            Self::String { min, max, choices } => {
                let mut value = Value::object([
                    ("type", Value::string("string")),
                    ("minLength", Value::u64(*min as u64)),
                    ("maxLength", Value::u64(*max as u64)),
                ]);
                if !choices.is_empty() {
                    value.insert(
                        "enum",
                        Value::Array(choices.iter().map(|s| Value::string(*s)).collect()),
                    );
                }
                value
            }
            Self::Array { items, min, max } => Value::object([
                ("type", Value::string("array")),
                ("items", items.to_json_at(depth + 1)),
                ("minItems", Value::u64(*min as u64)),
                ("maxItems", Value::u64(*max as u64)),
            ]),
            Self::Object {
                properties,
                additional,
            } => {
                let members = properties
                    .iter()
                    .map(|p| (String::from(p.name), p.schema.to_json_at(depth + 1)))
                    .collect();
                let required: Vec<Value> = properties
                    .iter()
                    .filter(|p| p.required)
                    .map(|p| Value::string(p.name))
                    .collect();
                Value::object([
                    ("type", Value::string("object")),
                    ("properties", Value::Object(members)),
                    ("required", Value::Array(required)),
                    ("additionalProperties", Value::Bool(*additional)),
                ])
            }
            Self::OneOf(options) => Value::object([(
                "oneOf",
                Value::Array(options.iter().map(|s| s.to_json_at(depth + 1)).collect()),
            )]),
        }
    }
}

fn pointer(prefix: &str, name: &str) -> String {
    format!("{prefix}/{}", name.replace('~', "~0").replace('/', "~1"))
}
