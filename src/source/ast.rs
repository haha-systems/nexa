use crate::{diagnostic::SourceSpan, value::ValueType};
use std::collections::BTreeMap;

#[derive(Debug, Clone)]
pub(super) struct Module {
    pub source_id: String,
    pub name: String,
    pub declarations: Vec<Declaration>,
}

#[derive(Debug, Clone)]
pub(super) struct Declaration {
    pub exported: bool,
    pub kind: CallableKind,
    pub name: String,
    pub parameters: Vec<Parameter>,
    pub result: ValueType,
    pub declared_effects: BTreeMap<String, SourceSpan>,
    pub body: Expr,
    pub span: SourceSpan,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CallableKind {
    Function,
    Action,
}

#[derive(Debug, Clone)]
pub(super) struct Parameter {
    pub name: String,
    pub ty: ValueType,
    pub span: SourceSpan,
}

#[derive(Debug, Clone)]
pub(super) struct Expr {
    pub kind: ExprKind,
    pub span: SourceSpan,
}

#[derive(Debug, Clone)]
pub(super) enum ExprKind {
    String(String),
    Unit,
    Name(String),
    Call {
        name: String,
        arguments: Vec<CallArgument>,
    },
    Concat(Box<Expr>, Box<Expr>),
}

#[derive(Debug, Clone)]
pub(super) struct CallArgument {
    pub name: Option<String>,
    pub expression: Expr,
    pub span: SourceSpan,
}
