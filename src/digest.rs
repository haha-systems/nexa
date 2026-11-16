use crate::diagnostic::{Error, Result};
use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::{borrow::Cow, fmt, str::FromStr};

/// The private string prevents invalid addresses from entering checked contracts.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String")]
pub struct Digest(String);

impl Digest {
    pub fn of_bytes(bytes: &[u8]) -> Self {
        Self(format!("sha256:{:x}", Sha256::digest(bytes)))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn verify(&self, bytes: &[u8]) -> Result<()> {
        if self == &Self::of_bytes(bytes) {
            Ok(())
        } else {
            Err(Error::new(
                "E_INTEGRITY",
                "content does not match its digest",
            ))
        }
    }
}

impl FromStr for Digest {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        if value.len() == 71
            && value.starts_with("sha256:")
            && value[7..]
                .bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
        {
            Ok(Self(value.to_owned()))
        } else {
            Err(Error::new(
                "E_DIGEST",
                "expected sha256: followed by 64 lowercase hex digits",
            ))
        }
    }
}

impl TryFrom<String> for Digest {
    type Error = Error;

    fn try_from(value: String) -> Result<Self> {
        value.parse()
    }
}

impl fmt::Display for Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl JsonSchema for Digest {
    fn schema_name() -> Cow<'static, str> {
        "Digest".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "string", "pattern": "^sha256:[0-9a-f]{64}$",
            "minLength": 71, "maxLength": 71
        })
    }
}
