//! Slice 0 source compilation: UTF-8 lexing, parsing, checking, and evaluation.
//!
//! The evaluator accepts only `CheckedModule`, whose constructor is private to
//! this module. Source spans retain byte offsets into the caller-supplied source.

mod ast;
mod checker;
mod lexer;
mod parser;

use crate::{
    contracts::CallableContract,
    diagnostic::{Error, Result},
    value::{Identifier, Value},
};
use checker::{
    CheckedCallTarget, CheckedCallable, CheckedExpr, CheckedExprKind, CheckedModuleData,
};
use std::collections::BTreeMap;

pub use crate::diagnostic::SourceSpan;

/// Host operations are invoked only from checked action calls. A host remains
/// responsible for authorization and for validating its own protected boundary.
pub trait Host {
    fn invoke(&mut self, operation: &str, args: &[Value], call_site: &SourceSpan) -> Result<Value>;

    /// Lets schedulers attach a graph node path to later host dispatches.
    fn set_node_path(&mut self, _path: &str) {}
}

/// Opaque parsed source, suitable only as input to [`check_parsed`].
#[derive(Debug, Clone)]
pub struct ParsedModule {
    inner: ast::Module,
}

impl ParsedModule {
    pub fn module_name(&self) -> &str {
        &self.inner.name
    }

    pub fn source_id(&self) -> &str {
        &self.inner.source_id
    }
}

/// Tokenize and parse one Slice 0 source module.
pub fn parse(source_id: &str, text: &str) -> Result<ParsedModule> {
    Ok(ParsedModule {
        inner: parser::parse(source_id, text)?,
    })
}

/// Parse and statically check source. Exactly one declaration must be exported.
pub fn check(source_id: &str, text: &str) -> Result<CheckedModule> {
    check_parsed(parse(source_id, text)?)
}

/// Check a module previously parsed with [`parse`].
pub fn check_parsed(parsed: ParsedModule) -> Result<CheckedModule> {
    let data = checker::check(parsed.inner)?;
    Ok(CheckedModule::from_data(data))
}

/// A module whose names, types, callable kinds, effects, and call graph have
/// passed the Slice 0 checker. This is the only source representation the
/// evaluator accepts.
#[derive(Debug, Clone)]
pub struct CheckedModule {
    module_name: String,
    exported_name: String,
    callables: BTreeMap<String, CheckedCallable>,
}

impl CheckedModule {
    fn from_data(data: CheckedModuleData) -> Self {
        Self {
            module_name: data.module_name,
            exported_name: data.exported_name,
            callables: data.callables,
        }
    }

    pub fn module_name(&self) -> &str {
        &self.module_name
    }

    /// Return the module's single public callable contract.
    pub fn exports(&self) -> Vec<CallableContract> {
        self.callables
            .get(&self.exported_name)
            .map(|callable| vec![callable.contract.clone()])
            .unwrap_or_default()
    }

    /// Return the public contract by export name.
    pub fn contract(&self, export: &str) -> Result<CallableContract> {
        if export != self.exported_name {
            return Err(Error::new(
                "E_UNKNOWN_EXPORT",
                format!("module {} does not export {export}", self.module_name),
            ));
        }
        self.callables
            .get(export)
            .map(|callable| callable.contract.clone())
            .ok_or_else(|| Error::new("E_UNKNOWN_EXPORT", format!("unknown export {export}")))
    }

    /// Evaluate a pure exported function. Actions are rejected because they
    /// need an explicit host and authorization context.
    pub fn evaluate(&self, export: &str, args: &[Value]) -> Result<Value> {
        let callable = self.checked_export(export)?;
        if callable.kind != ast::CallableKind::Function || !callable.contract.effects.is_empty() {
            return Err(Error::new(
                "E_CALL_KIND",
                format!("export {export} is effectful and requires evaluate_with"),
            ));
        }
        let mut evaluator = Evaluator {
            module: self,
            host: None,
        };
        evaluator.evaluate_callable(export, args)
    }

    /// Evaluate an exported function or action with a host for checked kernel
    /// calls. Pure function calls do not invoke the host.
    pub fn evaluate_with(
        &self,
        export: &str,
        args: &[Value],
        host: &mut dyn Host,
    ) -> Result<Value> {
        self.checked_export(export)?;
        let mut evaluator = Evaluator {
            module: self,
            host: Some(host),
        };
        evaluator.evaluate_callable(export, args)
    }

    /// Identifier-friendly form used by typed graph nodes.
    pub fn evaluate_export(
        &self,
        export: &Identifier,
        args: &[Value],
        host: &mut dyn Host,
    ) -> Result<Value> {
        self.evaluate_with(export.as_str(), args, host)
    }

    fn checked_export(&self, export: &str) -> Result<&CheckedCallable> {
        if export != self.exported_name {
            return Err(Error::new(
                "E_UNKNOWN_EXPORT",
                format!("module {} does not export {export}", self.module_name),
            ));
        }
        self.callables
            .get(export)
            .ok_or_else(|| Error::new("E_UNKNOWN_EXPORT", format!("unknown export {export}")))
    }
}

struct Evaluator<'a> {
    module: &'a CheckedModule,
    host: Option<&'a mut dyn Host>,
}

impl Evaluator<'_> {
    fn evaluate_callable(&mut self, name: &str, args: &[Value]) -> Result<Value> {
        let callable = self.module.callables.get(name).ok_or_else(|| {
            Error::new("E_EVALUATION", format!("unknown checked callable {name}"))
        })?;
        check_runtime_arguments(&callable.contract, args)?;
        let body = callable.body.clone();
        self.evaluate_expression(&body, args)
    }

    fn evaluate_expression(&mut self, expression: &CheckedExpr, args: &[Value]) -> Result<Value> {
        match &expression.kind {
            CheckedExprKind::String(value) => Ok(Value::String(value.clone())),
            CheckedExprKind::Unit => Ok(Value::Unit(())),
            CheckedExprKind::Parameter(index) => args.get(*index).cloned().ok_or_else(|| {
                Error::new("E_EVALUATION", "checked parameter index is out of range")
            }),
            CheckedExprKind::Concat(left, right) => {
                let left = self.evaluate_expression(left, args)?;
                let right = self.evaluate_expression(right, args)?;
                match (left, right) {
                    (Value::String(mut left), Value::String(right)) => {
                        left.push_str(&right);
                        Ok(Value::String(left))
                    }
                    _ => Err(Error::new(
                        "E_EVALUATION",
                        "checked concatenation received a non-string value",
                    )),
                }
            }
            CheckedExprKind::Call { target, arguments } => {
                let mut values = vec![None; arguments.len()];
                for argument in arguments {
                    let value = self.evaluate_expression(&argument.expression, args)?;
                    let slot = values.get_mut(argument.parameter_index).ok_or_else(|| {
                        Error::new("E_EVALUATION", "checked argument index is out of range")
                    })?;
                    *slot = Some(value);
                }
                let values: Vec<Value> = values
                    .into_iter()
                    .map(|value| {
                        value.ok_or_else(|| {
                            Error::new("E_EVALUATION", "checked call argument is missing")
                        })
                    })
                    .collect::<Result<_>>()?;
                match target {
                    CheckedCallTarget::Local(name) => self.evaluate_callable(name, &values),
                    CheckedCallTarget::Kernel(name) if name == "std.text.trim_ascii" => {
                        match values.as_slice() {
                            [Value::String(text)] => Ok(Value::String(
                                text.trim_matches(|ch: char| {
                                    matches!(ch, '\u{0009}'..='\u{000d}' | ' ')
                                })
                                .to_owned(),
                            )),
                            _ => Err(Error::new(
                                "E_EVALUATION",
                                "checked trim_ascii call received invalid arguments",
                            )),
                        }
                    }
                    CheckedCallTarget::Kernel(name) => {
                        let host = self.host.as_deref_mut().ok_or_else(|| {
                            Error::new(
                                "E_HOST_REQUIRED",
                                format!("kernel action {name} requires evaluate_with"),
                            )
                        })?;
                        let result = host.invoke(name, &values, &expression.span)?;
                        if result.value_type() != expression.ty {
                            return Err(Error::new(
                                "E_HOST_RESULT_TYPE",
                                format!(
                                    "host operation {name} returned {:?}, expected {:?}",
                                    result.value_type(),
                                    expression.ty
                                ),
                            ));
                        }
                        Ok(result)
                    }
                }
            }
        }
    }
}

fn check_runtime_arguments(contract: &CallableContract, args: &[Value]) -> Result<()> {
    if args.len() != contract.parameters.len() {
        return Err(Error::new(
            "E_ARGUMENT_COUNT",
            format!(
                "{} expects {} argument(s), got {}",
                contract.export_name,
                contract.parameters.len(),
                args.len()
            ),
        ));
    }
    for (parameter, value) in contract.parameters.iter().zip(args) {
        if value.value_type() != parameter.r#type {
            return Err(Error::new(
                "E_ARGUMENT_TYPE",
                format!(
                    "argument {} expects {:?}, got {:?}",
                    parameter.name,
                    parameter.r#type,
                    value.value_type()
                ),
            ));
        }
    }
    Ok(())
}
