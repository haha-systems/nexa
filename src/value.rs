use crate::diagnostic::{Error, Result};
use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Serialize};
use std::{borrow::Cow, fmt};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum ValueType {
    String,
    Unit,
}

/// `Unit(())` deliberately serializes its value as null, not as a missing field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", content = "value", deny_unknown_fields)]
pub enum Value {
    String(String),
    Unit(()),
}

impl Value {
    pub fn value_type(&self) -> ValueType {
        match self {
            Self::String(_) => ValueType::String,
            Self::Unit(()) => ValueType::Unit,
        }
    }
}

pub const RESERVED: &[&str] = &[
    "nexa",
    "module",
    "import",
    "export",
    "as",
    "type",
    "alias",
    "interface",
    "impl",
    "fn",
    "action",
    "effects",
    "let",
    "var",
    "state",
    "return",
    "if",
    "else",
    "match",
    "for",
    "in",
    "while",
    "break",
    "continue",
    "true",
    "false",
    "none",
    "unit",
    "and",
    "or",
    "not",
    "is",
    "main",
    "effect",
    "requires",
    "policy",
    "allow",
    "deny",
    "ask",
    "abstain",
    "when",
    "otherwise",
    "identity",
    "agent",
    "group",
    "grant",
    "revoke",
    "pulse",
    "run",
    "blueprint",
    "input",
    "output",
    "node",
    "connect",
    "after",
    "set",
    "memory",
    "on",
    "emit",
    "task",
    "spawn",
    "await",
    "scope",
    "defer",
];

pub fn identifier_shape(value: &str) -> bool {
    let mut chars = value.bytes();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == b'_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == b'_')
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String")]
pub struct Identifier(String);

impl Identifier {
    pub fn new(value: impl Into<String>) -> Result<Self> {
        let value = value.into();
        if identifier_shape(&value) && !RESERVED.contains(&value.as_str()) {
            Ok(Self(value))
        } else {
            Err(Error::new(
                "E_IDENTIFIER",
                format!("invalid or reserved identifier: {value}"),
            ))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for Identifier {
    type Error = Error;

    fn try_from(value: String) -> Result<Self> {
        Self::new(value)
    }
}

impl fmt::Display for Identifier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl JsonSchema for Identifier {
    fn schema_name() -> Cow<'static, str> {
        "Identifier".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "string", "pattern": "^[A-Za-z_][A-Za-z0-9_]*$",
            "not": {"anyOf": [{"enum": RESERVED}, {"pattern": "[\\r\\n]"}]}
        })
    }
}
