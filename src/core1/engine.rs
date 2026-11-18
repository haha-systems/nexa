//! Type checking and bounded evaluation for pure Core1 modules.

mod check;
mod run;
mod value;

use super::ast;
use crate::{
    diagnostic::{Error, Result},
    digest::Digest,
};
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;
use std::{collections::BTreeMap, fmt, sync::Arc};

pub use check::check;

pub struct CheckedModule {
    identity: Digest,
    name: String,
    imports: BTreeMap<String, Arc<CheckedModule>>,
    catalog: Catalog,
    functions: BTreeMap<String, CheckedFunction>,
    signature: Signature,
}

impl CheckedModule {
    pub fn signature(&self) -> &Signature {
        &self.signature
    }

    pub fn evaluate(&self, args: &Json, fuel: u64) -> Result<Json> {
        run::evaluate(self, args, fuel)
    }

    pub(super) fn evaluate_with_budget(
        &self,
        args: &Json,
        budget: &mut EvaluationBudget,
    ) -> Result<Json> {
        run::evaluate_with_budget(self, args, &mut budget.0)
    }

    pub(super) fn ports(&self) -> (String, Vec<(String, String)>, String) {
        self.signature.ports()
    }
}

/// One allocation and fuel budget shared by every Core1 node in a blueprint.
pub(super) struct EvaluationBudget(value::Budget);

impl EvaluationBudget {
    pub(super) fn new(fuel: u64) -> Self {
        Self(value::Budget::new(fuel))
    }

    pub(super) fn charge_copy(&mut self, bytes: usize) -> Result<()> {
        self.0.work(bytes)?;
        self.0.storage::<u8>(bytes)
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Signature {
    name: String,
    params: Vec<Field>,
    result: Type,
    types: BTreeMap<String, TypeShape>,
}

impl Signature {
    pub(super) fn ports(&self) -> (String, Vec<(String, String)>, String) {
        (
            self.name.clone(),
            self.params
                .iter()
                .map(|param| (param.name.clone(), param.ty.to_string()))
                .collect(),
            self.result.to_string(),
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind")]
enum Type {
    Unit,
    Bool,
    Int,
    Bytes,
    String,
    List { item: Box<Type> },
    Maybe { item: Box<Type> },
    Result { ok: Box<Type>, err: Box<Type> },
    Nominal(Nominal),
}

impl Type {
    fn list(item: Self) -> Self {
        Self::List {
            item: Box::new(item),
        }
    }

    fn maybe(item: Self) -> Self {
        Self::Maybe {
            item: Box::new(item),
        }
    }

    fn depth(&self) -> usize {
        match self {
            Self::List { item } | Self::Maybe { item } => 1 + item.depth(),
            Self::Result { ok, err } => 1 + ok.depth().max(err.depth()),
            _ => 1,
        }
    }

    fn record<'a>(&self, catalog: &'a Catalog) -> Option<&'a [Field]> {
        let Self::Nominal(id) = self else { return None };
        match catalog.shape(id)? {
            TypeShape::Record { fields } => Some(fields),
            TypeShape::Union { .. } => None,
        }
    }

    fn union<'a>(&'a self, catalog: &'a Catalog) -> Option<Union<'a>> {
        match self {
            Self::Maybe { item } => Some(Union::Maybe(item)),
            Self::Result { ok, err } => Some(Union::Result(ok, err)),
            Self::Nominal(id) => match catalog.shape(id)? {
                TypeShape::Union { variants } => Some(Union::Nominal(variants)),
                TypeShape::Record { .. } => None,
            },
            _ => None,
        }
    }
}

impl fmt::Display for Type {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unit => f.write_str("Unit"),
            Self::Bool => f.write_str("Bool"),
            Self::Int => f.write_str("Int"),
            Self::Bytes => f.write_str("Bytes"),
            Self::String => f.write_str("String"),
            Self::List { item } => write!(f, "List[{item}]"),
            Self::Maybe { item } => write!(f, "Maybe[{item}]"),
            Self::Result { ok, err } => write!(f, "Result[{ok}, {err}]"),
            Self::Nominal(id) => write!(f, "{}.{} ({})", id.module, id.name, id.identity),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Nominal {
    module: String,
    identity: Digest,
    name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Field {
    name: String,
    ty: Type,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind")]
enum TypeShape {
    Record { fields: Vec<Field> },
    Union { variants: Vec<Variant> },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Variant {
    name: String,
    fields: Vec<Field>,
}

/// One shared definition table per declaring memory in this import closure.
#[derive(Default)]
struct Catalog(BTreeMap<Digest, Arc<BTreeMap<String, TypeShape>>>);

impl Catalog {
    fn shape(&self, id: &Nominal) -> Option<&TypeShape> {
        self.0.get(&id.identity)?.get(&id.name)
    }
}

#[derive(Clone, Copy)]
enum Fields<'a> {
    None,
    One(&'static str, &'a Type),
    Declared(&'a [Field]),
}

impl<'a> Fields<'a> {
    fn len(self) -> usize {
        match self {
            Self::None => 0,
            Self::One(..) => 1,
            Self::Declared(fields) => fields.len(),
        }
    }

    fn get(self, index: usize) -> Option<(&'a str, &'a Type)> {
        match self {
            Self::None => None,
            Self::One(name, ty) => (index == 0).then_some((name, ty)),
            Self::Declared(fields) => fields.get(index).map(|f| (f.name.as_str(), &f.ty)),
        }
    }

    fn iter(self) -> impl Iterator<Item = (&'a str, &'a Type)> {
        (0..self.len()).filter_map(move |index| self.get(index))
    }
}

enum Union<'a> {
    Maybe(&'a Type),
    Result(&'a Type, &'a Type),
    Nominal(&'a [Variant]),
}

impl<'a> Union<'a> {
    fn len(&self) -> usize {
        match self {
            Self::Maybe(..) | Self::Result(..) => 2,
            Self::Nominal(variants) => variants.len(),
        }
    }

    fn get(&self, index: usize) -> Option<(&'a str, Fields<'a>)> {
        match (self, index) {
            (Self::Maybe(_), 0) => Some(("none", Fields::None)),
            (Self::Maybe(ty), 1) => Some(("some", Fields::One("value", ty))),
            (Self::Result(ok, _), 0) => Some(("ok", Fields::One("value", ok))),
            (Self::Result(_, err), 1) => Some(("err", Fields::One("error", err))),
            (Self::Nominal(variants), index) => variants
                .get(index)
                .map(|v| (v.name.as_str(), Fields::Declared(&v.fields))),
            _ => None,
        }
    }

    fn find(&self, name: &str) -> Option<(usize, Fields<'a>)> {
        (0..self.len()).find_map(|index| {
            let (variant, fields) = self.get(index)?;
            (variant == name).then_some((index, fields))
        })
    }
}

struct CheckedFunction {
    slots: usize,
    body: Block,
}

struct Block {
    slots: Vec<usize>,
    statements: Vec<Stmt>,
}

enum Stmt {
    Store(usize, Expr),
    Return(Expr),
    If(Expr, Block, Block),
    While(Expr, Block),
    Match(Expr, Vec<MatchArm>),
}

struct MatchArm {
    variant: Option<usize>,
    bindings: Vec<(usize, usize)>,
    body: Block,
}

enum Expr {
    Constant(value::Value),
    Local(usize),
    List(Vec<Expr>),
    Record(Arguments),
    Variant(usize, Arguments),
    Member(Box<Expr>, usize),
    Call(CallTarget, Arguments),
    Builtin(Builtin, Vec<Expr>),
    Unary(ast::Unary, Box<Expr>),
    Binary(ast::Binary, Box<Expr>, Box<Expr>),
}

/// Arguments retain source evaluation order and carry their destination slots.
struct Arguments(Vec<(usize, Expr)>);

enum CallTarget {
    Local(String),
    Imported(String, String),
}

#[derive(Clone, Copy)]
enum Builtin {
    BytesLen,
    BytesAt,
    BytesSlice,
    BytesConcat,
    Byte,
    BytesHex,
    EncodeUtf8,
    DecodeUtf8,
    StringLen,
    ListLen,
    ListGet,
    ListPush,
}

fn invariant() -> Error {
    Error::new(
        "E_CORE1_INVARIANT",
        "checked Core1 operation is inconsistent",
    )
}
