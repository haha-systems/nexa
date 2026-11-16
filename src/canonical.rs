//! Nexa's semantic JSON encoding. Array normalization is document-specific and
//! belongs to `contracts`, not to this general JSON encoder.

use crate::{
    diagnostic::{Error, Result},
    digest::Digest,
    value::identifier_shape,
};
use serde::{
    Deserialize, Deserializer,
    de::{Error as _, MapAccess, SeqAccess, Visitor},
};
use serde_json::{Map, Value};
use std::fmt;

struct Strict(Value);

impl<'de> Deserialize<'de> for Strict {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        struct StrictVisitor;
        impl<'de> Visitor<'de> for StrictVisitor {
            type Value = Strict;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("JSON with unique keys and signed 64-bit integers")
            }

            fn visit_bool<E: serde::de::Error>(self, v: bool) -> std::result::Result<Strict, E> {
                Ok(Strict(Value::Bool(v)))
            }

            fn visit_i64<E: serde::de::Error>(self, v: i64) -> std::result::Result<Strict, E> {
                Ok(Strict(Value::Number(v.into())))
            }

            fn visit_u64<E: serde::de::Error>(self, v: u64) -> std::result::Result<Strict, E> {
                let n = i64::try_from(v)
                    .map_err(|_| E::custom("integer exceeds signed 64-bit range"))?;
                Ok(Strict(Value::Number(n.into())))
            }

            fn visit_f64<E: serde::de::Error>(self, _: f64) -> std::result::Result<Strict, E> {
                Err(E::custom(
                    "JSON floats and negative zero are forbidden; use typed strings",
                ))
            }

            fn visit_str<E: serde::de::Error>(self, v: &str) -> std::result::Result<Strict, E> {
                Ok(Strict(Value::String(v.into())))
            }

            fn visit_string<E: serde::de::Error>(
                self,
                v: String,
            ) -> std::result::Result<Strict, E> {
                Ok(Strict(Value::String(v)))
            }

            fn visit_unit<E: serde::de::Error>(self) -> std::result::Result<Strict, E> {
                Ok(Strict(Value::Null))
            }

            fn visit_seq<A: SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> std::result::Result<Strict, A::Error> {
                let mut values = Vec::new();
                while let Some(value) = seq.next_element::<Strict>()? {
                    values.push(value.0);
                }
                Ok(Strict(Value::Array(values)))
            }

            fn visit_map<A: MapAccess<'de>>(
                self,
                mut map: A,
            ) -> std::result::Result<Strict, A::Error> {
                let mut values = Map::new();
                while let Some(key) = map.next_key::<String>()? {
                    if values.contains_key(&key) {
                        return Err(A::Error::custom(format!("duplicate key: {key}")));
                    }
                    let value = map.next_value::<Strict>()?;
                    values.insert(key, value.0);
                }
                Ok(Strict(Value::Object(values)))
            }
        }
        deserializer.deserialize_any(StrictVisitor)
    }
}

pub fn parse(bytes: &[u8]) -> Result<Value> {
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    let value = Strict::deserialize(&mut deserializer)
        .map_err(|e| Error::new("E_JSON", e.to_string()))?
        .0;

    deserializer
        .end()
        .map_err(|e| Error::new("E_JSON", e.to_string()))?;

    // Also check key/number rules for callers that only parse without encoding.
    encode(&value)?;
    Ok(value)
}

pub fn encode(value: &Value) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    emit(value, &mut bytes)?;
    Ok(bytes)
}

pub fn digest(value: &Value) -> Result<Digest> {
    Ok(Digest::of_bytes(&encode(value)?))
}

fn emit(value: &Value, bytes: &mut Vec<u8>) -> Result<()> {
    match value {
        Value::Null => bytes.extend_from_slice(b"null"),
        Value::Bool(true) => bytes.extend_from_slice(b"true"),
        Value::Bool(false) => bytes.extend_from_slice(b"false"),
        Value::Number(n) => {
            let integer = n
                .as_i64()
                .ok_or_else(|| Error::new("E_NUMBER", "expected signed 64-bit integer"))?;

            bytes.extend_from_slice(integer.to_string().as_bytes());
        }
        Value::String(s) => bytes.extend_from_slice(
            serde_json::to_string(s)
                .map_err(|e| Error::new("E_JSON", e.to_string()))?
                .as_bytes(),
        ),
        Value::Array(items) => {
            bytes.push(b'[');

            for (index, item) in items.iter().enumerate() {
                if index != 0 {
                    bytes.push(b',');
                }

                emit(item, bytes)?;
            }

            bytes.push(b']');
        }
        Value::Object(map) => {
            let mut entries: Vec<_> = map.iter().collect();

            entries.sort_by(|(left, _), (right, _)| left.as_bytes().cmp(right.as_bytes()));

            bytes.push(b'{');

            for (index, (key, item)) in entries.into_iter().enumerate() {
                if !identifier_shape(key) {
                    return Err(Error::new(
                        "E_KEY",
                        "semantic object keys must be ASCII schema keys or identifiers",
                    )
                    .at(key));
                }

                if index != 0 {
                    bytes.push(b',');
                }

                bytes.extend_from_slice(
                    serde_json::to_string(key)
                        .map_err(|e| Error::new("E_JSON", e.to_string()))?
                        .as_bytes(),
                );

                bytes.push(b':');

                emit(item, bytes)?;
            }

            bytes.push(b'}');
        }
    }
    Ok(())
}
