use super::{Catalog, Fields, Json, Type, TypeShape, Union, invariant};
use crate::diagnostic::{Error, Result};
use serde_json::Map;
use std::{mem::size_of, sync::Arc};

const MAX_DEPTH: usize = 64;
const MAX_ALLOCATION: usize = 64 * 1024 * 1024;
const ALLOCATION_OVERHEAD: usize = 2 * size_of::<usize>();
const HEX_DIGITS: &[u8; 16] = b"0123456789abcdef";

#[derive(Clone)]
pub(super) enum Value {
    Unit,
    Bool(bool),
    Int(i64),
    String(Arc<String>),
    Bytes(Arc<Vec<u8>>),
    List(Arc<Sequence>),
    Record(Arc<Sequence>),
    Variant(usize, Arc<Sequence>),
}

pub(super) struct Sequence {
    pub(super) items: Vec<Value>,
    depth: usize,
}

impl Value {
    fn depth(&self) -> usize {
        match self {
            Self::List(values) | Self::Record(values) | Self::Variant(_, values) => values.depth,
            _ => 0,
        }
    }
}

pub(super) fn sequence(items: Vec<Value>) -> Result<Arc<Sequence>> {
    let depth = 1 + items.iter().map(Value::depth).max().unwrap_or_default();
    check_depth(depth)?;
    Ok(Arc::new(Sequence { items, depth }))
}

pub(super) struct Budget {
    fuel: u64,
    input_work: usize,
    allocation: usize,
}

impl Budget {
    pub(super) fn new(fuel: u64) -> Self {
        Self {
            fuel,
            input_work: MAX_ALLOCATION,
            allocation: MAX_ALLOCATION,
        }
    }

    pub(super) fn step(&mut self) -> Result<()> {
        self.work(1)
    }

    pub(super) fn work(&mut self, amount: usize) -> Result<()> {
        self.fuel = self
            .fuel
            .checked_sub(amount as u64)
            .ok_or_else(|| Error::new("E_CORE1_FUEL", "Core1 evaluation exhausted its fuel"))?;
        Ok(())
    }

    pub(super) fn alloc(&mut self, amount: usize) -> Result<()> {
        self.allocation = self
            .allocation
            .checked_sub(amount)
            .ok_or_else(allocation_limit)?;
        Ok(())
    }

    pub(super) fn storage<T>(&mut self, len: usize) -> Result<()> {
        let bytes = len
            .checked_mul(size_of::<T>())
            .and_then(|bytes| bytes.checked_add(ALLOCATION_OVERHEAD))
            .ok_or_else(allocation_limit)?;
        self.alloc(bytes)
    }

    // Validate inputs before evaluation. A caller's fuel setting cannot mask a
    // malformed argument, and large inputs still have a bounded validation cost.
    fn read_input(&mut self, amount: usize) -> Result<()> {
        self.input_work = self.input_work.checked_sub(amount).ok_or_else(|| {
            Error::new("E_CORE1_LIMIT", "Core1 input exceeds the validation limit")
        })?;
        Ok(())
    }
}

pub(super) fn decode(
    ty: &Type,
    json: &Json,
    catalog: &Catalog,
    budget: &mut Budget,
    depth: usize,
) -> Result<Value> {
    check_depth(depth)?;
    budget.read_input(1)?;
    match ty {
        Type::Unit if json.is_null() => Ok(Value::Unit),
        Type::Unit => Err(invalid("expected null for Unit")),
        Type::Bool => json
            .as_bool()
            .map(Value::Bool)
            .ok_or_else(|| invalid("expected a JSON boolean for Bool")),
        Type::Int => decode_int(json).map(Value::Int),
        Type::String => {
            let text = json
                .as_str()
                .ok_or_else(|| invalid("expected a JSON string for String"))?;
            budget.read_input(text.len())?;
            budget.storage::<u8>(text.len())?;
            budget.storage::<String>(1)?;
            Ok(Value::String(Arc::new(text.to_owned())))
        }
        Type::Bytes => {
            let text = json.as_str().ok_or_else(invalid_hex)?;
            let len = hex_len(text)?;
            budget.read_input(text.len())?;
            budget.storage::<u8>(len)?;
            budget.storage::<Vec<u8>>(1)?;
            Ok(Value::Bytes(Arc::new(decode_hex(text, len)?)))
        }
        Type::List { item } => {
            let items = json
                .as_array()
                .ok_or_else(|| invalid("expected a JSON array for List"))?;
            budget.storage::<Sequence>(1)?;
            budget.storage::<Value>(items.len())?;
            let mut values = Vec::with_capacity(items.len());
            for element in items {
                values.push(decode(item, element, catalog, budget, depth + 1)?);
            }
            Ok(Value::List(sequence(values)?))
        }
        Type::Nominal(id) => match catalog.shape(id).ok_or_else(invariant)? {
            TypeShape::Record { fields } => {
                decode_fields(Fields::Declared(fields), json, catalog, budget, depth)
                    .map(Value::Record)
            }
            TypeShape::Union { variants } => {
                decode_variant(Union::Nominal(variants), json, catalog, budget, depth)
            }
        },
        Type::Maybe { item } => decode_variant(Union::Maybe(item), json, catalog, budget, depth),
        Type::Result { ok, err } => {
            decode_variant(Union::Result(ok, err), json, catalog, budget, depth)
        }
    }
}

fn decode_int(json: &Json) -> Result<i64> {
    let invalid_int = || invalid("expected a canonical signed 64-bit decimal string for Int");
    let text = json.as_str().ok_or_else(invalid_int)?;
    if text == "0" {
        return Ok(0);
    }
    let digits = text.strip_prefix('-').unwrap_or(text);
    if text.len() > 20 || !matches!(digits.as_bytes().first(), Some(b'1'..=b'9')) {
        return Err(invalid_int());
    }
    text.parse().map_err(|_| invalid_int())
}

fn decode_fields(
    fields: Fields<'_>,
    json: &Json,
    catalog: &Catalog,
    budget: &mut Budget,
    depth: usize,
) -> Result<Arc<Sequence>> {
    let invalid_fields = || invalid("expected an object containing exactly the declared fields");
    let object = json
        .as_object()
        .filter(|object| object.len() == fields.len())
        .ok_or_else(invalid_fields)?;
    budget.storage::<Sequence>(1)?;
    budget.storage::<Value>(fields.len())?;
    let mut values = Vec::with_capacity(fields.len());
    for (name, ty) in fields.iter() {
        budget.read_input(name.len())?;
        let json = object.get(name).ok_or_else(invalid_fields)?;
        values.push(decode(ty, json, catalog, budget, depth + 1)?);
    }
    sequence(values)
}

fn decode_variant(
    union: Union<'_>,
    json: &Json,
    catalog: &Catalog,
    budget: &mut Budget,
    depth: usize,
) -> Result<Value> {
    let invalid_variant = || invalid("expected a tagged union with variant and fields entries");
    let object = json
        .as_object()
        .filter(|object| object.len() == 2)
        .ok_or_else(invalid_variant)?;
    let name = object
        .get("variant")
        .and_then(Json::as_str)
        .ok_or_else(invalid_variant)?;
    let payload = object.get("fields").ok_or_else(invalid_variant)?;
    budget.read_input(union.len().saturating_mul(name.len().max(1)))?;
    let (tag, fields) = union
        .find(name)
        .ok_or_else(|| invalid("unknown tagged union variant"))?;
    let values = decode_fields(fields, payload, catalog, budget, depth)?;
    Ok(Value::Variant(tag, values))
}

pub(super) fn from_hex(text: &str, budget: &mut Budget) -> Result<Vec<u8>> {
    let len = hex_len(text)?;
    budget.work(text.len())?;
    budget.storage::<u8>(len)?;
    decode_hex(text, len)
}

fn hex_len(text: &str) -> Result<usize> {
    if !text.len().is_multiple_of(2) {
        return Err(invalid_hex());
    }
    Ok(text.len() / 2)
}

fn decode_hex(text: &str, len: usize) -> Result<Vec<u8>> {
    let mut values = Vec::with_capacity(len);
    for &[high, low] in text.as_bytes().as_chunks::<2>().0 {
        let high = hex_digit(high).ok_or_else(invalid_hex)?;
        let low = hex_digit(low).ok_or_else(invalid_hex)?;
        values.push(high * 16 + low);
    }
    Ok(values)
}

fn hex_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}

pub(super) fn encode(
    ty: &Type,
    value: &Value,
    catalog: &Catalog,
    budget: &mut Budget,
    depth: usize,
) -> Result<Json> {
    check_depth(depth)?;
    budget.step()?;
    match (ty, value) {
        (Type::Unit, Value::Unit) => Ok(Json::Null),
        (Type::Bool, Value::Bool(value)) => Ok(Json::Bool(*value)),
        (Type::Int, Value::Int(value)) => {
            budget.storage::<u8>(20)?;
            Ok(Json::String(value.to_string()))
        }
        (Type::String, Value::String(value)) => {
            budget.work(value.len())?;
            budget.storage::<u8>(value.len())?;
            Ok(Json::String(value.as_ref().clone()))
        }
        (Type::Bytes, Value::Bytes(value)) => {
            let len = value.len().checked_mul(2).ok_or_else(allocation_limit)?;
            budget.work(len)?;
            budget.storage::<u8>(len)?;
            let mut text = String::with_capacity(len);
            for &byte in value.iter() {
                text.push(char::from(HEX_DIGITS[usize::from(byte >> 4)]));
                text.push(char::from(HEX_DIGITS[usize::from(byte & 0x0f)]));
            }
            Ok(Json::String(text))
        }
        (Type::List { item }, Value::List(values)) => {
            budget.storage::<Json>(values.items.len())?;
            let mut items = Vec::with_capacity(values.items.len());
            for value in &values.items {
                items.push(encode(item, value, catalog, budget, depth + 1)?);
            }
            Ok(Json::Array(items))
        }
        (ty, Value::Record(values)) => {
            let fields = ty.record(catalog).ok_or_else(invariant)?;
            encode_fields(Fields::Declared(fields), values, catalog, budget, depth)
        }
        (ty, Value::Variant(tag, values)) => {
            let union = ty.union(catalog).ok_or_else(invariant)?;
            let (name, fields) = union.get(*tag).ok_or_else(invariant)?;
            budget.work(name.len())?;
            budget.storage::<u8>(name.len())?;
            let mut object = output_object(2, budget)?;
            object.insert("variant".into(), Json::String(name.into()));
            object.insert(
                "fields".into(),
                encode_fields(fields, values, catalog, budget, depth)?,
            );
            Ok(Json::Object(object))
        }
        _ => Err(invariant()),
    }
}

fn encode_fields(
    fields: Fields<'_>,
    values: &Sequence,
    catalog: &Catalog,
    budget: &mut Budget,
    depth: usize,
) -> Result<Json> {
    if fields.len() != values.items.len() {
        return Err(invariant());
    }
    let mut object = output_object(fields.len(), budget)?;
    for ((name, ty), value) in fields.iter().zip(&values.items) {
        budget.work(name.len())?;
        budget.storage::<u8>(name.len())?;
        object.insert(name.into(), encode(ty, value, catalog, budget, depth + 1)?);
    }
    Ok(Json::Object(object))
}

fn output_object(len: usize, budget: &mut Budget) -> Result<Map<String, Json>> {
    // JSON objects can allocate nodes with capacity beyond their populated entries.
    let entries = len.checked_mul(2).ok_or_else(allocation_limit)?;
    budget.storage::<(String, Json)>(entries)?;
    if len != 0 {
        budget.alloc(1024)?;
    }
    Ok(Map::new())
}

pub(super) fn equal(left: &Value, right: &Value, budget: &mut Budget) -> Result<bool> {
    budget.step()?;
    match (left, right) {
        (Value::Unit, Value::Unit) => Ok(true),
        (Value::Bool(left), Value::Bool(right)) => Ok(left == right),
        (Value::Int(left), Value::Int(right)) => Ok(left == right),
        (Value::String(left), Value::String(right)) => {
            if Arc::ptr_eq(left, right) {
                return Ok(true);
            }
            equal_bytes(left.as_bytes(), right.as_bytes(), budget)
        }
        (Value::Bytes(left), Value::Bytes(right)) => {
            if Arc::ptr_eq(left, right) {
                return Ok(true);
            }
            equal_bytes(left, right, budget)
        }
        (Value::List(left), Value::List(right)) | (Value::Record(left), Value::Record(right)) => {
            equal_sequences(left, right, budget)
        }
        (Value::Variant(a, left), Value::Variant(b, right)) if a == b => {
            equal_sequences(left, right, budget)
        }
        _ => Ok(false),
    }
}

fn equal_bytes(left: &[u8], right: &[u8], budget: &mut Budget) -> Result<bool> {
    if left.len() != right.len() {
        return Ok(false);
    }
    budget.work(left.len())?;
    Ok(left == right)
}

fn equal_sequences(
    left: &Arc<Sequence>,
    right: &Arc<Sequence>,
    budget: &mut Budget,
) -> Result<bool> {
    if Arc::ptr_eq(left, right) {
        return Ok(true);
    }
    if left.items.len() != right.items.len() {
        return Ok(false);
    }
    for (left, right) in left.items.iter().zip(&right.items) {
        if !equal(left, right, budget)? {
            return Ok(false);
        }
    }
    Ok(true)
}

fn check_depth(depth: usize) -> Result<()> {
    if depth > MAX_DEPTH {
        return Err(Error::new(
            "E_CORE1_LIMIT",
            "Core1 value nesting exceeds the limit",
        ));
    }
    Ok(())
}

fn allocation_limit() -> Error {
    Error::new(
        "E_CORE1_LIMIT",
        "Core1 evaluation exceeds the allocation limit",
    )
}

fn invalid(message: &str) -> Error {
    Error::new("E_CORE1_VALUE", message)
}

fn invalid_hex() -> Error {
    invalid("expected a lowercase hexadecimal string with complete byte pairs for Bytes")
}
