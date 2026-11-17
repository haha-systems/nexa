//! Syntax of the experimental pure Core1 source profile.

#[derive(Debug, Clone)]
pub struct Module {
    pub name: String,
    pub imports: Vec<String>,
    pub types: Vec<TypeDecl>,
    pub functions: Vec<Function>,
}

#[derive(Debug, Clone)]
pub struct TypeDecl {
    pub name: String,
    pub exported: bool,
    pub kind: TypeDeclKind,
}

#[derive(Debug, Clone)]
pub enum TypeDeclKind {
    Record(Vec<Field>),
    Union(Vec<Variant>),
}

#[derive(Debug, Clone)]
pub struct Variant {
    pub name: String,
    pub fields: Vec<Field>,
}

#[derive(Debug, Clone)]
pub struct Field {
    pub name: String,
    pub ty: TypeExpr,
}

#[derive(Debug, Clone)]
pub enum TypeExpr {
    Named(String),
    Generic(String, Vec<TypeExpr>),
}

#[derive(Debug, Clone)]
pub struct Function {
    pub name: String,
    pub exported: bool,
    pub params: Vec<Field>,
    pub result: TypeExpr,
    pub body: Vec<Stmt>,
}

#[derive(Debug, Clone)]
pub enum Stmt {
    Bind {
        name: String,
        mutable: bool,
        ty: Option<TypeExpr>,
        value: Expr,
    },
    Assign {
        name: String,
        value: Expr,
    },
    Return(Expr),
    If {
        condition: Expr,
        then_body: Vec<Stmt>,
        else_body: Vec<Stmt>,
    },
    While {
        condition: Expr,
        body: Vec<Stmt>,
    },
    Match {
        value: Expr,
        arms: Vec<Arm>,
    },
}

#[derive(Debug, Clone)]
pub struct Arm {
    pub variant: String,
    pub bindings: Vec<(String, String)>,
    pub body: Vec<Stmt>,
}

#[derive(Debug, Clone)]
pub enum Expr {
    Unit,
    Bool(bool),
    Int(i64),
    Bytes(Vec<u8>),
    String(String),
    Name(String),
    List(Vec<Expr>),
    Record(String, Vec<(String, Expr)>),
    Call(String, Vec<Expr>),
    NamedCall(String, Vec<(String, Expr)>),
    Member(Box<Expr>, String),
    Unary(Unary, Box<Expr>),
    Binary(Binary, Box<Expr>, Box<Expr>),
}

#[derive(Debug, Clone, Copy)]
pub enum Unary {
    Negate,
    Not,
}

#[derive(Debug, Clone, Copy)]
pub enum Binary {
    Add,
    Subtract,
    Multiply,
    Divide,
    Remainder,
    Equal,
    NotEqual,
    Less,
    LessEqual,
    Greater,
    GreaterEqual,
    And,
    Or,
}
