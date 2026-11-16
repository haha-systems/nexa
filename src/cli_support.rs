use nexa::{
    canonical,
    contracts::{self, Port, WireDocument},
    diagnostic::{Error, Result},
    digest::Digest,
    value::{Identifier, Value, ValueType},
};
use serde::Serialize;
use serde_json::Value as Json;
use std::{collections::BTreeMap, fs, path::Path};

pub fn read(path: &Path) -> Result<Vec<u8>> {
    fs::read(path).map_err(|e| Error::new("E_IO", e.to_string()).at(path.display().to_string()))
}

pub fn text(path: &Path) -> Result<String> {
    String::from_utf8(read(path)?)
        .map_err(|e| Error::new("E_UTF8", e.to_string()).at(path.display().to_string()))
}

pub fn document<T: WireDocument>(path: &Path) -> Result<T> {
    contracts::decode(canonical::parse(&read(path)?)?)
}

pub fn json<T: Serialize>(value: &T) -> Result<Json> {
    serde_json::to_value(value).map_err(|e| Error::new("E_CONTRACT", e.to_string()))
}

pub fn pretty<T: Serialize>(value: &T) -> Result<Vec<u8>> {
    let mut bytes =
        serde_json::to_vec_pretty(value).map_err(|e| Error::new("E_JSON", e.to_string()))?;
    bytes.push(b'\n');
    Ok(bytes)
}

pub fn write_bytes(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)
            .map_err(|e| Error::new("E_IO", e.to_string()).at(parent.display().to_string()))?;
    }
    fs::write(path, bytes)
        .map_err(|e| Error::new("E_IO", e.to_string()).at(path.display().to_string()))
}

pub fn write_document<T: Serialize>(path: &Path, value: &T) -> Result<Digest> {
    let bytes = canonical::encode(&json(value)?)?;
    write_bytes(path, &bytes)?;
    Ok(Digest::of_bytes(&bytes))
}

/// Decode plain input JSON against the complete declared external interface.
/// `null` is accepted only for an explicitly declared Unit input.
pub fn inputs(value: Json, ports: &[Port]) -> Result<BTreeMap<Identifier, Value>> {
    let object = value.as_object().ok_or_else(|| {
        Error::new(
            "E_INPUT",
            "invocation input must be an object with exactly the declared fields",
        )
    })?;
    if object.len() != ports.len() {
        return Err(Error::new(
            "E_INPUT",
            "missing or extra invocation input fields",
        ));
    }
    let mut result = BTreeMap::new();
    for port in ports {
        let raw = object
            .get(port.name.as_str())
            .ok_or_else(|| Error::new("E_INPUT", format!("missing input {}", port.name)))?;
        let typed = match (port.r#type, raw) {
            (ValueType::String, Json::String(s)) => Value::String(s.clone()),
            (ValueType::Unit, Json::Null) => Value::Unit(()),
            _ => {
                return Err(Error::new(
                    "E_INPUT",
                    format!(
                        "wrong type for input {}: expected {:?}",
                        port.name, port.r#type
                    ),
                ));
            }
        };
        result.insert(port.name.clone(), typed);
    }
    Ok(result)
}

/// Pagination cursors bind both semantic revision and selected node path.
pub fn cursor_offset(program: &Digest, node: Option<&str>, cursor: Option<&str>) -> Result<usize> {
    let Some(cursor) = cursor else {
        return Ok(0);
    };
    let scope = Digest::of_bytes(node.unwrap_or("").as_bytes());
    let parts: Vec<_> = cursor.split('.').collect();
    if parts.len() != 3 || parts[0] != program.as_str() || parts[1] != scope.as_str() {
        return Err(Error::new(
            "E_CURSOR",
            "inspection cursor belongs to another revision or node",
        ));
    }
    parts[2]
        .parse::<usize>()
        .map_err(|_| Error::new("E_CURSOR", "invalid inspection cursor offset"))
}

/// Decorate an already bounded library page; never request or truncate it twice.
pub fn page(
    mut inspection: Json,
    program: &Digest,
    node: Option<&str>,
    fields: &[String],
) -> Result<Json> {
    let scope = Digest::of_bytes(node.unwrap_or("").as_bytes());
    let object = inspection
        .as_object_mut()
        .ok_or_else(|| Error::new("E_INSPECT", "inspection must return an object"))?;
    let next = object
        .get("next_offset")
        .or_else(|| object.get("children").and_then(|v| v.get("next_offset")))
        .and_then(Json::as_u64)
        .map(|offset| format!("{program}.{scope}.{offset}"));
    object.insert("subject".into(), json(program)?);
    object.insert("protocol_version".into(), Json::String("0.3".into()));
    object.insert("next_cursor".into(), next.map_or(Json::Null, Json::String));
    object.remove("next_offset");
    if !fields.is_empty() {
        let preserved = [
            "subject",
            "protocol_version",
            "profile",
            "schema",
            "next_cursor",
        ];
        for field in fields {
            if !object.contains_key(field) {
                return Err(Error::new(
                    "E_FIELD",
                    format!("unknown inspection field: {field}"),
                ));
            }
        }
        let omitted: Vec<String> = object
            .keys()
            .filter(|key| !preserved.contains(&key.as_str()) && !fields.contains(key))
            .cloned()
            .collect();
        object.retain(|key, _| preserved.contains(&key.as_str()) || fields.contains(key));
        object.insert("omitted_fields".into(), json(&omitted)?);
    }
    Ok(inspection)
}
