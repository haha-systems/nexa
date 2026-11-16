use crate::{
    contracts::{CallableContract, CallableKind as ContractCallableKind, CapabilityRequest, Port},
    diagnostic::{Error, Result, SourceSpan},
    kernel::KernelDescriptor,
    value::{Identifier, ValueType},
};
use std::collections::{BTreeMap, BTreeSet};

use super::ast::{CallableKind, Declaration, Expr, ExprKind, Module};

const MAX_CALL_GRAPH_DEPTH: usize = 256;

#[derive(Debug, Clone)]
pub(super) struct CheckedCallable {
    pub contract: CallableContract,
    pub kind: CallableKind,
    pub body: CheckedExpr,
}

#[derive(Debug, Clone)]
pub(super) struct CheckedExpr {
    pub kind: CheckedExprKind,
    pub ty: ValueType,
    pub span: SourceSpan,
    pub effects: BTreeSet<String>,
    pub required_bindings: BTreeSet<String>,
    pub capability_requirements: BTreeSet<CapabilityRequest>,
}

#[derive(Debug, Clone)]
pub(super) enum CheckedExprKind {
    String(String),
    Unit,
    Parameter(usize),
    Call {
        target: CheckedCallTarget,
        arguments: Vec<CheckedArgument>,
    },
    Concat(Box<CheckedExpr>, Box<CheckedExpr>),
}

#[derive(Debug, Clone)]
pub(super) struct CheckedArgument {
    pub parameter_index: usize,
    pub expression: CheckedExpr,
}

#[derive(Debug, Clone)]
pub(super) enum CheckedCallTarget {
    Local(String),
    Kernel(String),
}

pub(super) struct CheckedModuleData {
    pub module_name: String,
    pub exported_name: String,
    pub callables: BTreeMap<String, CheckedCallable>,
}

struct ResolvedCall {
    target: CheckedCallTarget,
    kind: CallableKind,
    parameters: Vec<Port>,
    result_type: ValueType,
    effects: BTreeSet<String>,
    required_bindings: BTreeSet<String>,
    capability_requirements: BTreeSet<CapabilityRequest>,
}

pub(super) fn check(module: Module) -> Result<CheckedModuleData> {
    Checker::new(module)?.run()
}

struct Checker {
    module_name: String,
    declarations: BTreeMap<String, Declaration>,
    kernel: BTreeMap<String, CallableContract>,
    registered_effects: BTreeSet<String>,
    checked: BTreeMap<String, CheckedCallable>,
    visiting: BTreeSet<String>,
    exported_name: String,
}

impl Checker {
    fn new(module: Module) -> Result<Self> {
        let mut declarations = BTreeMap::new();
        let mut exported = Vec::new();
        for declaration in module.declarations {
            if declarations
                .insert(declaration.name.clone(), declaration.clone())
                .is_some()
            {
                return Err(at(
                    "E_DUPLICATE",
                    format!("duplicate callable {}", declaration.name),
                    &declaration.span,
                ));
            }
            if declaration.exported {
                exported.push(declaration.name.clone());
            }
        }
        if exported.len() != 1 {
            return Err(Error::new(
                "E_EXPORT_COUNT",
                format!(
                    "a slice0 source module must export exactly one callable; found {}",
                    exported.len()
                ),
            ));
        }

        let descriptor = KernelDescriptor::bundled()?;
        let kernel: BTreeMap<String, CallableContract> = descriptor
            .operations
            .into_iter()
            .map(|operation| (operation.name, operation.contract))
            .collect();
        let registered_effects = kernel
            .values()
            .flat_map(|contract| contract.effects.iter().cloned())
            .collect();

        let checker = Self {
            module_name: module.name,
            declarations,
            kernel,
            registered_effects,
            checked: BTreeMap::new(),
            visiting: BTreeSet::new(),
            exported_name: exported.remove(0),
        };
        checker.reject_recursion()?;
        Ok(checker)
    }

    fn run(mut self) -> Result<CheckedModuleData> {
        let names: Vec<_> = self.declarations.keys().cloned().collect();
        for name in names {
            self.check_declaration(&name)?;
        }
        Ok(CheckedModuleData {
            module_name: self.module_name,
            exported_name: self.exported_name,
            callables: self.checked,
        })
    }

    fn check_declaration(&mut self, name: &str) -> Result<()> {
        if self.checked.contains_key(name) {
            return Ok(());
        }
        if !self.visiting.insert(name.to_owned()) {
            let span = self
                .declarations
                .get(name)
                .map(|declaration| declaration.span.clone());
            return Err(match span {
                Some(span) => at(
                    "E_RECURSION",
                    "recursive calls are unsupported in slice0",
                    &span,
                ),
                None => Error::new("E_RECURSION", "recursive calls are unsupported in slice0"),
            });
        }

        let declaration = self
            .declarations
            .get(name)
            .expect("checked names come from the declaration map")
            .clone();
        for (effect, span) in &declaration.declared_effects {
            if !self.registered_effects.contains(effect) {
                self.visiting.remove(name);
                return Err(at(
                    "E_UNKNOWN_EFFECT",
                    format!("effect {effect} is not registered in the Slice 0 kernel"),
                    span,
                ));
            }
        }
        let mut parameters = BTreeMap::new();
        for (index, parameter) in declaration.parameters.iter().enumerate() {
            if parameters
                .insert(parameter.name.clone(), (index, parameter.ty))
                .is_some()
            {
                self.visiting.remove(name);
                return Err(at(
                    "E_DUPLICATE",
                    format!("duplicate parameter {}", parameter.name),
                    &parameter.span,
                ));
            }
        }

        let body = match self.check_expression(&declaration.body, &parameters, declaration.kind) {
            Ok(body) => body,
            Err(error) => {
                self.visiting.remove(name);
                return Err(error);
            }
        };
        if body.ty != declaration.result {
            self.visiting.remove(name);
            return Err(at(
                "E_RETURN_TYPE",
                format!(
                    "{} returns {:?}, but its declared result is {:?}",
                    declaration.name, body.ty, declaration.result
                ),
                &declaration.body.span,
            ));
        }

        let effects = match declaration.kind {
            CallableKind::Function => {
                if !body.effects.is_empty() {
                    self.visiting.remove(name);
                    return Err(at(
                        "E_EFFECT_IN_FN",
                        "a pure function cannot call code with effects",
                        &declaration.body.span,
                    ));
                }
                Vec::new()
            }
            CallableKind::Action => {
                let declared_effects: BTreeSet<_> =
                    declaration.declared_effects.keys().cloned().collect();
                let missing: Vec<_> = body
                    .effects
                    .difference(&declared_effects)
                    .cloned()
                    .collect();
                if !missing.is_empty() {
                    self.visiting.remove(name);
                    return Err(at(
                        "E_EFFECT_SET",
                        format!(
                            "action {} does not declare transitively reachable effect(s): {}",
                            declaration.name,
                            missing.join(", ")
                        ),
                        &declaration.span,
                    ));
                }
                declared_effects.into_iter().collect()
            }
        };

        let mut contract = CallableContract {
            export_name: Identifier::new(declaration.name.clone())?,
            kind: match declaration.kind {
                CallableKind::Function => ContractCallableKind::Function,
                CallableKind::Action => ContractCallableKind::Action,
            },
            parameters: declaration
                .parameters
                .iter()
                .map(|parameter| {
                    Ok(Port {
                        name: Identifier::new(parameter.name.clone())?,
                        r#type: parameter.ty,
                    })
                })
                .collect::<Result<Vec<_>>>()?,
            result_type: declaration.result,
            effects,
            required_bindings: body.required_bindings.iter().cloned().collect(),
            capability_requirements: body.capability_requirements.iter().cloned().collect(),
        };
        // The contract's effect and binding collections have canonical order.
        contract.effects.sort();
        contract.required_bindings.sort();
        contract.capability_requirements.sort();

        self.visiting.remove(name);
        self.checked.insert(
            name.to_owned(),
            CheckedCallable {
                contract,
                kind: declaration.kind,
                body,
            },
        );
        Ok(())
    }

    fn check_expression(
        &mut self,
        expression: &Expr,
        parameters: &BTreeMap<String, (usize, ValueType)>,
        caller_kind: CallableKind,
    ) -> Result<CheckedExpr> {
        match &expression.kind {
            ExprKind::String(value) => Ok(CheckedExpr {
                kind: CheckedExprKind::String(value.clone()),
                ty: ValueType::String,
                span: expression.span.clone(),
                effects: BTreeSet::new(),
                required_bindings: BTreeSet::new(),
                capability_requirements: BTreeSet::new(),
            }),
            ExprKind::Unit => Ok(CheckedExpr {
                kind: CheckedExprKind::Unit,
                ty: ValueType::Unit,
                span: expression.span.clone(),
                effects: BTreeSet::new(),
                required_bindings: BTreeSet::new(),
                capability_requirements: BTreeSet::new(),
            }),
            ExprKind::Name(name) => {
                if let Some((index, ty)) = parameters.get(name) {
                    Ok(CheckedExpr {
                        kind: CheckedExprKind::Parameter(*index),
                        ty: *ty,
                        span: expression.span.clone(),
                        effects: BTreeSet::new(),
                        required_bindings: BTreeSet::new(),
                        capability_requirements: BTreeSet::new(),
                    })
                } else if self.declarations.contains_key(name) || self.kernel.contains_key(name) {
                    Err(at(
                        "E_CALLABLE_VALUE",
                        format!("callable {name} cannot be used as a value in slice0"),
                        &expression.span,
                    ))
                } else {
                    Err(at(
                        "E_UNKNOWN_NAME",
                        format!("unknown value {name}"),
                        &expression.span,
                    ))
                }
            }
            ExprKind::Concat(left, right) => {
                let left = self.check_expression(left, parameters, caller_kind)?;
                let right = self.check_expression(right, parameters, caller_kind)?;
                if left.ty != ValueType::String || right.ty != ValueType::String {
                    return Err(at(
                        "E_TYPE",
                        format!(
                            "string concatenation requires String operands, got {:?} and {:?}",
                            left.ty, right.ty
                        ),
                        &expression.span,
                    ));
                }
                let (effects, bindings, capabilities) = expression_summary(&left);
                let (right_effects, right_bindings, right_capabilities) =
                    expression_summary(&right);
                let mut effects = effects;
                let mut bindings = bindings;
                let mut capabilities = capabilities;
                effects.extend(right_effects);
                bindings.extend(right_bindings);
                capabilities.extend(right_capabilities);
                Ok(CheckedExpr {
                    kind: CheckedExprKind::Concat(Box::new(left), Box::new(right)),
                    ty: ValueType::String,
                    span: expression.span.clone(),
                    effects,
                    required_bindings: bindings,
                    capability_requirements: capabilities,
                })
            }
            ExprKind::Call { name, arguments } => {
                let resolved = self.resolve_call(name, &expression.span)?;
                if caller_kind == CallableKind::Function && resolved.kind == CallableKind::Action {
                    return Err(at(
                        "E_EFFECT_IN_FN",
                        format!("a pure function cannot call action {name}"),
                        &expression.span,
                    ));
                }
                if arguments.len() != resolved.parameters.len() {
                    return Err(at(
                        "E_ARGUMENT_COUNT",
                        format!(
                            "call to {name} expects {} argument(s), got {}",
                            resolved.parameters.len(),
                            arguments.len()
                        ),
                        &expression.span,
                    ));
                }
                let has_named = arguments.iter().any(|argument| argument.name.is_some());
                let has_positional = arguments.iter().any(|argument| argument.name.is_none());
                if has_named && has_positional {
                    return Err(at(
                        "E_ARGUMENT_NAMING",
                        "named and positional arguments cannot be mixed",
                        &expression.span,
                    ));
                }
                let mut parameter_indices = Vec::with_capacity(arguments.len());
                let mut seen_parameters = BTreeSet::new();
                for (position, argument) in arguments.iter().enumerate() {
                    let parameter_index = if let Some(argument_name) = &argument.name {
                        resolved
                            .parameters
                            .iter()
                            .position(|parameter| parameter.name.as_str() == argument_name)
                            .ok_or_else(|| {
                                at(
                                    "E_ARGUMENT_NAME",
                                    format!("{argument_name} is not a parameter of {name}"),
                                    &argument.span,
                                )
                            })?
                    } else {
                        position
                    };
                    if !seen_parameters.insert(parameter_index) {
                        return Err(at(
                            "E_DUPLICATE",
                            format!(
                                "parameter {} is supplied more than once",
                                resolved.parameters[parameter_index].name
                            ),
                            &argument.span,
                        ));
                    }
                    parameter_indices.push(parameter_index);
                }
                let mut checked_arguments = Vec::with_capacity(arguments.len());
                for (argument, parameter_index) in arguments.iter().zip(parameter_indices) {
                    let expected = &resolved.parameters[parameter_index];
                    let checked =
                        self.check_expression(&argument.expression, parameters, caller_kind)?;
                    if checked.ty != expected.r#type {
                        return Err(at(
                            "E_ARGUMENT_TYPE",
                            format!(
                                "argument to {name}.{} expects {:?}, got {:?}",
                                expected.name, expected.r#type, checked.ty
                            ),
                            &argument.expression.span,
                        ));
                    }
                    checked_arguments.push(CheckedArgument {
                        parameter_index,
                        expression: checked,
                    });
                }

                let mut all_effects = resolved.effects;
                let mut all_bindings = resolved.required_bindings;
                let mut all_capabilities = resolved.capability_requirements;
                for argument in &checked_arguments {
                    let (argument_effects, argument_bindings, argument_capabilities) =
                        expression_summary(&argument.expression);
                    all_effects.extend(argument_effects);
                    all_bindings.extend(argument_bindings);
                    all_capabilities.extend(argument_capabilities);
                }

                Ok(CheckedExpr {
                    kind: CheckedExprKind::Call {
                        target: resolved.target,
                        arguments: checked_arguments,
                    },
                    ty: resolved.result_type,
                    span: expression.span.clone(),
                    effects: BTreeSet::new(),
                    required_bindings: BTreeSet::new(),
                    capability_requirements: BTreeSet::new(),
                }
                .with_summary(all_effects, all_bindings, all_capabilities))
            }
        }
    }

    fn resolve_call(&mut self, name: &str, span: &SourceSpan) -> Result<ResolvedCall> {
        if name.starts_with("std.") {
            let Some(contract) = self.kernel.get(name).cloned() else {
                return Err(at(
                    "E_UNKNOWN_NAME",
                    format!("unknown std operation {name}"),
                    span,
                ));
            };
            let kind = match contract.kind {
                ContractCallableKind::Function => CallableKind::Function,
                ContractCallableKind::Action => CallableKind::Action,
            };
            return Ok(ResolvedCall {
                target: CheckedCallTarget::Kernel(name.to_owned()),
                kind,
                parameters: contract.parameters,
                result_type: contract.result_type,
                effects: contract.effects.into_iter().collect(),
                required_bindings: contract.required_bindings.into_iter().collect(),
                capability_requirements: contract.capability_requirements.into_iter().collect(),
            });
        }

        if name.contains('.') {
            return Err(at(
                "E_UNKNOWN_NAME",
                format!("qualified source call {name} is outside slice0; only std is implicit"),
                span,
            ));
        }

        let Some(declaration) = self.declarations.get(name).cloned() else {
            return Err(at(
                "E_UNKNOWN_NAME",
                format!("unknown callable {name}"),
                span,
            ));
        };
        self.check_declaration(name)?;
        let checked = self
            .checked
            .get(name)
            .expect("declaration is checked before its signature is used");
        Ok(ResolvedCall {
            target: CheckedCallTarget::Local(name.to_owned()),
            kind: declaration.kind,
            parameters: checked.contract.parameters.clone(),
            result_type: checked.contract.result_type,
            effects: checked.contract.effects.iter().cloned().collect(),
            required_bindings: checked.contract.required_bindings.iter().cloned().collect(),
            capability_requirements: checked
                .contract
                .capability_requirements
                .iter()
                .cloned()
                .collect(),
        })
    }

    fn reject_recursion(&self) -> Result<()> {
        let mut graph: BTreeMap<String, Vec<(String, SourceSpan)>> = BTreeMap::new();
        for (name, declaration) in &self.declarations {
            let mut edges = Vec::new();
            collect_local_calls(&declaration.body, &self.declarations, &mut edges);
            graph.insert(name.clone(), edges);
        }
        let mut states = BTreeMap::<String, u8>::new();
        for name in self.declarations.keys() {
            visit_call_graph(name, &graph, &mut states, 0)?;
        }
        Ok(())
    }
}

impl CheckedExpr {
    fn with_summary(
        mut self,
        effects: BTreeSet<String>,
        required_bindings: BTreeSet<String>,
        capability_requirements: BTreeSet<CapabilityRequest>,
    ) -> Self {
        self.effects = effects;
        self.required_bindings = required_bindings;
        self.capability_requirements = capability_requirements;
        self
    }
}

fn expression_summary(
    expression: &CheckedExpr,
) -> (
    BTreeSet<String>,
    BTreeSet<String>,
    BTreeSet<CapabilityRequest>,
) {
    match &expression.kind {
        CheckedExprKind::Call { arguments, .. } => {
            let mut effects = expression.effects.clone();
            let mut bindings = expression.required_bindings.clone();
            let mut capabilities = expression.capability_requirements.clone();
            for argument in arguments {
                let (child_effects, child_bindings, child_capabilities) =
                    expression_summary(&argument.expression);
                effects.extend(child_effects);
                bindings.extend(child_bindings);
                capabilities.extend(child_capabilities);
            }
            (effects, bindings, capabilities)
        }
        CheckedExprKind::Concat(left, right) => {
            let (mut effects, mut bindings, mut capabilities) = expression_summary(left);
            let (other_effects, other_bindings, other_capabilities) = expression_summary(right);
            effects.extend(other_effects);
            bindings.extend(other_bindings);
            capabilities.extend(other_capabilities);
            (effects, bindings, capabilities)
        }
        _ => (BTreeSet::new(), BTreeSet::new(), BTreeSet::new()),
    }
}

fn collect_local_calls(
    expression: &Expr,
    declarations: &BTreeMap<String, Declaration>,
    edges: &mut Vec<(String, SourceSpan)>,
) {
    match &expression.kind {
        ExprKind::Call { name, arguments } => {
            if !name.contains('.') && declarations.contains_key(name) {
                edges.push((name.clone(), expression.span.clone()));
            }
            for argument in arguments {
                collect_local_calls(&argument.expression, declarations, edges);
            }
        }
        ExprKind::Concat(left, right) => {
            collect_local_calls(left, declarations, edges);
            collect_local_calls(right, declarations, edges);
        }
        ExprKind::String(_) | ExprKind::Unit | ExprKind::Name(_) => {}
    }
}

fn visit_call_graph(
    name: &str,
    graph: &BTreeMap<String, Vec<(String, SourceSpan)>>,
    states: &mut BTreeMap<String, u8>,
    depth: usize,
) -> Result<()> {
    match states.get(name).copied() {
        Some(2) => return Ok(()),
        Some(1) => return Ok(()),
        _ => {}
    }
    states.insert(name.to_owned(), 1);
    if let Some(edges) = graph.get(name) {
        for (target, span) in edges {
            if states.get(target) == Some(&1) {
                return Err(at(
                    "E_RECURSION",
                    format!("recursive call from {name} to {target} is unsupported in slice0"),
                    span,
                ));
            }
            if depth >= MAX_CALL_GRAPH_DEPTH {
                return Err(at(
                    "E_NESTING_LIMIT",
                    format!("source helper call depth exceeds {MAX_CALL_GRAPH_DEPTH}"),
                    span,
                ));
            }
            visit_call_graph(target, graph, states, depth + 1)?;
        }
    }
    states.insert(name.to_owned(), 2);
    Ok(())
}

fn at(code: &str, message: impl Into<String>, span: &SourceSpan) -> Error {
    Error {
        diagnostic: crate::diagnostic::Diagnostic {
            code: code.to_owned(),
            message: message.into(),
            path: None,
            span: Some(span.clone()),
        },
    }
}
