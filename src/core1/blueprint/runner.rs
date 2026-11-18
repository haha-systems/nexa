//! Evaluation of a rechecked Core1 blueprint.

use super::{CheckedProgram, Endpoint, Identifier};
use crate::{
    core1::{INPUT_LIMIT, MAX_FUEL, engine::EvaluationBudget},
    diagnostic::{Error, Result},
};
use serde_json::{Map, Value as Json};
use std::{collections::BTreeMap, io, mem::size_of};

pub(super) fn run_verified(checked: &CheckedProgram, input: &Json, fuel: u64) -> Result<Json> {
    if !(1..=MAX_FUEL).contains(&fuel) {
        return Err(Error::new(
            "E_LIMIT",
            format!("fuel must be between 1 and {MAX_FUEL}"),
        ));
    }
    let program = &checked.program;
    let input_fields = input
        .as_object()
        .filter(|fields| {
            fields.len() == program.inputs.len()
                && program
                    .inputs
                    .iter()
                    .all(|port| fields.contains_key(port.name.as_str()))
        })
        .ok_or_else(|| {
            Error::new(
                "E_INPUT",
                "provide exactly the declared Core1 blueprint inputs",
            )
        })?;
    wire_limit(input)?;

    let mut inbound = BTreeMap::<&Identifier, Vec<(&Identifier, &Endpoint)>>::new();
    let mut output = None;
    for wire in &program.connections {
        match &wire.to {
            Endpoint::Node { node, port } => {
                inbound.entry(node).or_default().push((port, &wire.from))
            }
            Endpoint::Output { .. } => output = Some(&wire.from),
            Endpoint::Input { .. } => return Err(invariant()),
        }
    }

    let mut budget = EvaluationBudget::new(fuel);
    let mut results = BTreeMap::<&Identifier, Json>::new();
    for node in &checked.order {
        let module = checked.nodes.get(node).ok_or_else(invariant)?;
        let mut args = Map::new();
        if let Some(wires) = inbound.get(node) {
            for (param, producer) in wires {
                let value = match producer {
                    Endpoint::Input { port } => input_fields.get(port.as_str()),
                    Endpoint::Node { node, .. } => results.get(node),
                    Endpoint::Output { .. } => None,
                }
                .ok_or_else(invariant)?;
                budget.charge_copy(copy_cost(value, 0)? + param.as_str().len())?;
                args.insert(param.as_str().to_owned(), value.clone());
            }
        }
        let value = module
            .evaluate_with_budget(&Json::Object(args), &mut budget)
            .map_err(|error| error.at(node.to_string()))?;
        wire_limit(&value).map_err(|error| error.at(node.to_string()))?;
        results.insert(node, value);
    }

    match output {
        Some(Endpoint::Node { node, .. }) => results.remove(node).ok_or_else(invariant),
        _ => Err(invariant()),
    }
}

/// Count clone costs before copying a wire value, including object keys and containers.
fn copy_cost(value: &Json, depth: usize) -> Result<usize> {
    if depth > 128 {
        return Err(Error::new(
            "E_LIMIT",
            "Core1 wire value nesting exceeds the limit",
        ));
    }
    let mut cost = size_of::<Json>();
    match value {
        Json::String(text) => cost += text.len(),
        Json::Array(items) => {
            for item in items {
                cost += copy_cost(item, depth + 1)?;
            }
        }
        Json::Object(object) => {
            for (key, item) in object {
                cost += size_of::<String>() + key.len() + copy_cost(item, depth + 1)?;
            }
        }
        _ => {}
    }
    Ok(cost)
}

fn wire_limit(value: &Json) -> Result<()> {
    // Validate depth before calling the serializer on an in-memory input.
    copy_cost(value, 0)?;
    serde_json::to_writer(WireLimit(INPUT_LIMIT), value)
        .map_err(|_| Error::new("E_LIMIT", "Core1 wire value exceeds 1 MiB"))
}

struct WireLimit(usize);

impl io::Write for WireLimit {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0 = self
            .0
            .checked_sub(bytes.len())
            .ok_or_else(|| io::Error::other("Core1 wire byte limit exceeded"))?;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn invariant() -> Error {
    Error::new(
        "E_CORE1_INVARIANT",
        "checked Core1 blueprint wiring is inconsistent",
    )
}
