use super::*;
use std::collections::BTreeSet;

const MAX_NESTING: usize = 64;

pub fn check(
    module: ast::Module,
    imports: BTreeMap<String, Arc<CheckedModule>>,
    identity: Digest,
) -> Result<CheckedModule> {
    let mut local_types = BTreeMap::new();
    for declaration in &module.types {
        if primitive(&declaration.name).is_some()
            || matches!(declaration.name.as_str(), "List" | "Maybe" | "Result")
            || imports.contains_key(&declaration.name)
            || local_types
                .insert(declaration.name.as_str(), declaration.exported)
                .is_some()
        {
            return Err(error(format!(
                "duplicate or reserved type name {}",
                declaration.name
            )));
        }
    }

    let resolver = Resolver {
        name: &module.name,
        identity: &identity,
        local_types,
        imports: &imports,
    };

    let mut shapes = BTreeMap::new();
    for declaration in &module.types {
        let shape = match &declaration.kind {
            ast::TypeDeclKind::Record(fields) => TypeShape::Record {
                fields: resolver.fields(fields, declaration.exported)?,
            },
            ast::TypeDeclKind::Union(variants) => {
                let mut names = BTreeSet::new();
                if variants.is_empty() {
                    return Err(error("a union requires at least one variant"));
                }
                let variants = variants
                    .iter()
                    .map(|variant| {
                        if variant.name == "_" || !names.insert(&variant.name) {
                            return Err(error(format!(
                                "duplicate or reserved variant {}.{}",
                                declaration.name, variant.name
                            )));
                        }
                        Ok(Variant {
                            name: variant.name.clone(),
                            fields: resolver.fields(&variant.fields, declaration.exported)?,
                        })
                    })
                    .collect::<Result<_>>()?;
                TypeShape::Union { variants }
            }
        };
        shapes.insert(declaration.name.clone(), shape);
    }

    let mut catalog = Catalog::default();
    for import in imports.values() {
        for (id, shapes) in &import.catalog.0 {
            catalog
                .0
                .entry(id.clone())
                .or_insert_with(|| shapes.clone());
        }
    }
    let shapes = Arc::new(shapes);
    catalog.0.insert(identity.clone(), shapes.clone());

    let mut headers = BTreeMap::new();
    let mut exports = module.functions.iter().filter(|function| function.exported);
    let export = exports
        .next()
        .filter(|_| exports.next().is_none())
        .ok_or_else(|| {
            Error::new(
                "E_CORE1_EXPORT",
                "a Core1 memory requires exactly one exported function",
            )
        })?;
    for function in &module.functions {
        let header = Header {
            params: resolver.fields(&function.params, function.exported)?,
            result: resolver.resolve(&function.result, function.exported, 0)?,
        };
        if headers.insert(function.name.as_str(), header).is_some() {
            return Err(error(format!("duplicate function {}", function.name)));
        }
    }

    let mut functions = BTreeMap::new();
    for function in &module.functions {
        let header = &headers[function.name.as_str()];
        let mut checker = Checker {
            resolver: &resolver,
            catalog: &catalog,
            headers: &headers,
            result: &header.result,
            scopes: Vec::new(),
            slots: 0,
        };
        let (body, returns, _) = checker
            .block(&function.body, &header.params, 0)
            .map_err(|error| error.at(format!("{}.{}", module.name, function.name)))?;
        if !returns && header.result != Type::Unit {
            return Err(error(format!(
                "function {} can finish without returning {}",
                function.name, header.result
            )));
        }
        functions.insert(
            function.name.clone(),
            CheckedFunction {
                slots: checker.slots,
                body,
            },
        );
    }

    let header = &headers[export.name.as_str()];
    let signature = Signature {
        name: export.name.clone(),
        params: header.params.clone(),
        result: header.result.clone(),
        types: shapes
            .iter()
            .filter(|(name, _)| resolver.local_types.get(name.as_str()) == Some(&true))
            .map(|(name, shape)| (name.clone(), shape.clone()))
            .collect(),
    };

    Ok(CheckedModule {
        identity,
        name: module.name,
        imports,
        catalog,
        functions,
        signature,
    })
}

struct Resolver<'a> {
    name: &'a str,
    identity: &'a Digest,
    local_types: BTreeMap<&'a str, bool>,
    imports: &'a BTreeMap<String, Arc<CheckedModule>>,
}

impl Resolver<'_> {
    fn nominal(&self, path: &str, public: bool) -> Option<Type> {
        let (module, identity, name) = if let Some((alias, name)) = path.split_once('.') {
            let import = self.imports.get(alias)?;
            if !import.signature.types.contains_key(name) {
                return None;
            }
            (&import.name[..], &import.identity, name)
        } else {
            let exported = self.local_types.get(path)?;
            if public && !exported {
                return None;
            }
            (self.name, self.identity, path)
        };
        Some(Type::Nominal(Nominal {
            module: module.to_owned(),
            identity: identity.clone(),
            name: name.to_owned(),
        }))
    }

    fn resolve(&self, syntax: &ast::TypeExpr, public: bool, depth: usize) -> Result<Type> {
        nesting(depth)?;
        match syntax {
            ast::TypeExpr::Named(name) => primitive(name)
                .or_else(|| self.nominal(name, public))
                .ok_or_else(|| error(format!("unknown or private type {name}"))),
            ast::TypeExpr::Generic(name, args) => match (name.as_str(), &args[..]) {
                ("List", [item]) => Ok(Type::list(self.resolve(item, public, depth + 1)?)),
                ("Maybe", [item]) => Ok(Type::maybe(self.resolve(item, public, depth + 1)?)),
                ("Result", [ok, err]) => Ok(Type::Result {
                    ok: Box::new(self.resolve(ok, public, depth + 1)?),
                    err: Box::new(self.resolve(err, public, depth + 1)?),
                }),
                _ => Err(error(format!(
                    "unknown generic type or wrong number of arguments for {name}"
                ))),
            },
        }
    }

    fn fields(&self, fields: &[ast::Field], public: bool) -> Result<Vec<Field>> {
        let mut names = BTreeSet::new();
        fields
            .iter()
            .map(|field| {
                if !names.insert(&field.name) {
                    return Err(error(format!(
                        "duplicate field or parameter {}",
                        field.name
                    )));
                }
                Ok(Field {
                    name: field.name.clone(),
                    ty: self.resolve(&field.ty, public, 0)?,
                })
            })
            .collect()
    }
}

fn primitive(name: &str) -> Option<Type> {
    match name {
        "Unit" => Some(Type::Unit),
        "Bool" => Some(Type::Bool),
        "Int" => Some(Type::Int),
        "Bytes" => Some(Type::Bytes),
        "String" => Some(Type::String),
        _ => None,
    }
}

struct Header {
    params: Vec<Field>,
    result: Type,
}

struct Local {
    index: usize,
    ty: Type,
    mutable: bool,
}

struct TypedExpr {
    ty: Type,
    expr: Expr,
}

#[derive(Clone, Copy)]
enum SourceArgs<'a> {
    Positional(&'a [ast::Expr]),
    Named(&'a [(String, ast::Expr)]),
}

impl SourceArgs<'_> {
    fn len(self) -> usize {
        match self {
            Self::Positional(values) => values.len(),
            Self::Named(values) => values.len(),
        }
    }
}

struct Checker<'a> {
    resolver: &'a Resolver<'a>,
    catalog: &'a Catalog,
    headers: &'a BTreeMap<&'a str, Header>,
    result: &'a Type,
    scopes: Vec<BTreeMap<String, Local>>,
    slots: usize,
}

impl Checker<'_> {
    fn local(&self, name: &str) -> Option<&Local> {
        self.scopes.iter().rev().find_map(|scope| scope.get(name))
    }

    fn bind(&mut self, name: &str, ty: Type, mutable: bool) -> Result<usize> {
        let scope = self.scopes.last_mut().ok_or_else(invariant)?;
        if scope.contains_key(name) {
            return Err(error(format!("duplicate binding {name}")));
        }
        let index = self.slots;
        self.slots += 1;
        scope.insert(name.to_owned(), Local { index, ty, mutable });
        Ok(index)
    }

    fn block(
        &mut self,
        statements: &[ast::Stmt],
        bindings: &[Field],
        depth: usize,
    ) -> Result<(Block, bool, Vec<usize>)> {
        nesting(depth)?;
        self.scopes.push(BTreeMap::new());
        let bindings = bindings
            .iter()
            .map(|binding| self.bind(&binding.name, binding.ty.clone(), false))
            .collect::<Result<_>>()?;
        let mut returns = false;
        let mut checked = Vec::with_capacity(statements.len());
        for statement in statements {
            let (statement, statement_returns) = self.statement(statement, depth + 1)?;
            checked.push(statement);
            returns |= statement_returns;
        }
        let scope = self.scopes.pop().ok_or_else(invariant)?;
        let slots = scope.into_values().map(|local| local.index).collect();

        Ok((
            Block {
                slots,
                statements: checked,
            },
            returns,
            bindings,
        ))
    }

    fn statement(&mut self, statement: &ast::Stmt, depth: usize) -> Result<(Stmt, bool)> {
        match statement {
            ast::Stmt::Bind {
                name,
                mutable,
                ty,
                value,
            } => {
                let expected = ty
                    .as_ref()
                    .map(|ty| self.resolver.resolve(ty, false, 0))
                    .transpose()?;
                let value = self.expression(value, expected.as_ref(), depth)?;
                let slot = self.bind(name, value.ty, *mutable)?;
                Ok((Stmt::Store(slot, value.expr), false))
            }
            ast::Stmt::Assign { name, value } => {
                let local = self
                    .local(name)
                    .ok_or_else(|| error(format!("unknown binding {name}")))?;
                if !local.mutable {
                    return Err(error(format!("binding {name} is immutable")));
                }
                let value = self.expression(value, Some(&local.ty), depth)?.expr;
                Ok((Stmt::Store(local.index, value), false))
            }
            ast::Stmt::Return(value) => {
                let value = self.expression(value, Some(self.result), depth)?;
                Ok((Stmt::Return(value.expr), true))
            }
            ast::Stmt::If {
                condition,
                then_body,
                else_body,
            } => {
                let condition = self.expression(condition, Some(&Type::Bool), depth)?.expr;
                let (then_body, then_returns, _) = self.block(then_body, &[], depth)?;
                let (else_body, else_returns, _) = self.block(else_body, &[], depth)?;
                Ok((
                    Stmt::If(condition, then_body, else_body),
                    then_returns && else_returns,
                ))
            }
            ast::Stmt::While { condition, body } => {
                let stays_in_loop = matches!(condition, ast::Expr::Bool(true));
                let condition = self.expression(condition, Some(&Type::Bool), depth)?.expr;
                let (body, _, _) = self.block(body, &[], depth)?;
                Ok((Stmt::While(condition, body), stays_in_loop))
            }
            ast::Stmt::Match { value, arms } => self.match_statement(value, arms, depth),
        }
    }

    fn match_statement(
        &mut self,
        value: &ast::Expr,
        arms: &[ast::Arm],
        depth: usize,
    ) -> Result<(Stmt, bool)> {
        let value = self.expression(value, None, depth)?;
        let union = value
            .ty
            .union(self.catalog)
            .ok_or_else(|| error(format!("match requires a tagged union, got {}", value.ty)))?;
        let mut seen = BTreeSet::new();
        let mut fallback = false;
        let mut returns = true;
        let mut checked = Vec::with_capacity(arms.len());
        for arm in arms {
            if fallback {
                return Err(error("the wildcard match arm must be last"));
            }
            let (variant, fields) = if arm.variant == "_" {
                if !arm.bindings.is_empty() {
                    return Err(error("a wildcard match arm cannot bind fields"));
                }
                if seen.len() == union.len() {
                    return Err(error("a wildcard match arm overlaps all preceding arms"));
                }
                fallback = true;
                (None, Fields::None)
            } else {
                let name = match arm.variant.rsplit_once('.') {
                    Some((owner, name)) if self.pattern_owner(owner, &value.ty) => name,
                    Some(_) => {
                        return Err(error(format!(
                            "pattern {} belongs to a different type",
                            arm.variant
                        )));
                    }
                    None => &arm.variant,
                };
                let (index, fields) = union
                    .find(name)
                    .ok_or_else(|| error(format!("unknown match variant {}", arm.variant)))?;
                if !seen.insert(index) {
                    return Err(error(format!("duplicate match variant {}", arm.variant)));
                }
                (Some(index), fields)
            };
            let field_names = fields
                .iter()
                .enumerate()
                .map(|(index, (name, ty))| (name, (index, ty)))
                .collect::<BTreeMap<_, _>>();
            let mut referenced = BTreeSet::new();
            let (positions, bindings): (Vec<_>, Vec<_>) = arm
                .bindings
                .iter()
                .map(|(field, name)| {
                    let &(position, ty) = field_names.get(field.as_str()).ok_or_else(|| {
                        error(format!("pattern {} has no field {field}", arm.variant))
                    })?;
                    if !referenced.insert(position) {
                        return Err(error(format!("duplicate pattern field {field}")));
                    }
                    Ok((
                        position,
                        Field {
                            name: name.clone(),
                            ty: ty.clone(),
                        },
                    ))
                })
                .collect::<Result<Vec<_>>>()?
                .into_iter()
                .unzip();
            let (body, arm_returns, slots) = self.block(&arm.body, &bindings, depth)?;
            returns &= arm_returns;
            checked.push(MatchArm {
                variant,
                bindings: positions.into_iter().zip(slots).collect(),
                body,
            });
        }
        if !fallback && seen.len() < union.len() {
            let missing = (0..union.len())
                .find(|index| !seen.contains(index))
                .and_then(|index| union.get(index))
                .ok_or_else(invariant)?;
            return Err(error(format!("nonexhaustive match, missing {}", missing.0)));
        }
        Ok((Stmt::Match(value.expr, checked), returns))
    }

    fn pattern_owner(&self, owner: &str, ty: &Type) -> bool {
        match (owner, ty) {
            ("Maybe", Type::Maybe { .. }) | ("Result", Type::Result { .. }) => true,
            _ => self.resolver.nominal(owner, false).as_ref() == Some(ty),
        }
    }

    fn expression(
        &self,
        expr: &ast::Expr,
        expected: Option<&Type>,
        depth: usize,
    ) -> Result<TypedExpr> {
        nesting(depth)?;
        let depth = depth + 1;
        let (ty, expr) = match expr {
            ast::Expr::Unit => (Type::Unit, Expr::Constant(value::Value::Unit)),
            ast::Expr::Bool(value) => (Type::Bool, Expr::Constant(value::Value::Bool(*value))),
            ast::Expr::Int(value) => (Type::Int, Expr::Constant(value::Value::Int(*value))),
            ast::Expr::String(value) => (
                Type::String,
                Expr::Constant(value::Value::String(Arc::new(value.clone()))),
            ),
            ast::Expr::Bytes(value) => (
                Type::Bytes,
                Expr::Constant(value::Value::Bytes(Arc::new(value.clone()))),
            ),
            ast::Expr::Name(name) => {
                let local = self
                    .local(name)
                    .ok_or_else(|| error(format!("unknown binding {name}")))?;
                (local.ty.clone(), Expr::Local(local.index))
            }
            ast::Expr::List(items) => self.list(items, expected, depth)?,
            ast::Expr::Record(name, fields) => {
                let ty = self
                    .resolver
                    .nominal(name, false)
                    .ok_or_else(|| error(format!("unknown record type {name}")))?;
                let shape = ty
                    .record(self.catalog)
                    .ok_or_else(|| error(format!("{name} is a tagged union")))?;
                let arguments = self.arguments(
                    SourceArgs::Named(fields),
                    Fields::Declared(shape),
                    depth,
                    name,
                )?;
                (ty, Expr::Record(arguments))
            }
            ast::Expr::Call(name, arguments) => {
                self.call(name, SourceArgs::Positional(arguments), expected, depth)?
            }
            ast::Expr::NamedCall(name, arguments) => {
                self.call(name, SourceArgs::Named(arguments), expected, depth)?
            }
            ast::Expr::Member(base, name) => {
                return self.member(base, name, expected, depth);
            }
            ast::Expr::Unary(operator, inner) => {
                let ty = match operator {
                    ast::Unary::Negate => Type::Int,
                    ast::Unary::Not => Type::Bool,
                };
                let operand = self.expression(inner, Some(&ty), depth)?.expr;
                (ty, Expr::Unary(*operator, Box::new(operand)))
            }
            ast::Expr::Binary(operator, left, right) => {
                self.binary(*operator, left, right, expected, depth)?
            }
        };
        typed(ty, expr, expected)
    }

    fn list(
        &self,
        items: &[ast::Expr],
        expected: Option<&Type>,
        depth: usize,
    ) -> Result<(Type, Expr)> {
        let mut output = Vec::with_capacity(items.len());
        let item_type = if let Some(Type::List { item }) = expected {
            *item.clone()
        } else {
            let (first, _) = items
                .split_first()
                .ok_or_else(|| error("an empty list requires an expected List type"))?;
            let first = self.expression(first, None, depth)?;
            output.push(first.expr);
            first.ty
        };
        for item in &items[output.len()..] {
            output.push(self.expression(item, Some(&item_type), depth)?.expr);
        }
        Ok((Type::list(item_type), Expr::List(output)))
    }

    fn member(
        &self,
        base: &ast::Expr,
        name: &str,
        expected: Option<&Type>,
        depth: usize,
    ) -> Result<TypedExpr> {
        if let Some(mut path) = self.type_path(base) {
            path.push('.');
            path.push_str(name);
            if let Some((ty, expr)) =
                self.variant(&path, SourceArgs::Positional(&[]), expected, depth)?
            {
                return typed(ty, expr, expected);
            }
        }
        let base = self.expression(base, None, depth)?;
        let fields = base
            .ty
            .record(self.catalog)
            .ok_or_else(|| error(format!("member access requires a record, got {}", base.ty)))?;
        let (index, field) = fields
            .iter()
            .enumerate()
            .find(|(_, f)| f.name == name)
            .ok_or_else(|| error(format!("{} has no field {name}", base.ty)))?;
        typed(
            field.ty.clone(),
            Expr::Member(Box::new(base.expr), index),
            expected,
        )
    }

    fn type_path(&self, expr: &ast::Expr) -> Option<String> {
        match expr {
            ast::Expr::Name(name) if self.local(name).is_none() => Some(name.clone()),
            ast::Expr::Member(base, name) => match base.as_ref() {
                ast::Expr::Name(alias) if self.local(alias).is_none() => {
                    Some(format!("{alias}.{name}"))
                }
                _ => None,
            },
            _ => None,
        }
    }

    fn binary(
        &self,
        operator: ast::Binary,
        left: &ast::Expr,
        right: &ast::Expr,
        expected: Option<&Type>,
        depth: usize,
    ) -> Result<(Type, Expr)> {
        use ast::Binary::*;
        let (required, result) = match operator {
            And | Or => (Some(&Type::Bool), Some(Type::Bool)),
            Equal | NotEqual => (None, Some(Type::Bool)),
            Less | LessEqual | Greater | GreaterEqual => (Some(&Type::Int), Some(Type::Bool)),
            Add => (
                expected.filter(|ty| matches!(ty, Type::Int | Type::String)),
                None,
            ),
            Subtract | Multiply | Divide | Remainder => (Some(&Type::Int), Some(Type::Int)),
        };
        let left = self.expression(left, required, depth)?;
        if matches!(operator, Add) && !matches!(left.ty, Type::Int | Type::String) {
            return Err(error(format!(
                "addition requires Int or String, got {}",
                left.ty
            )));
        }
        let right = self.expression(right, Some(&left.ty), depth)?;
        Ok((
            result.unwrap_or(left.ty),
            Expr::Binary(operator, Box::new(left.expr), Box::new(right.expr)),
        ))
    }

    fn call(
        &self,
        name: &str,
        args: SourceArgs<'_>,
        expected: Option<&Type>,
        depth: usize,
    ) -> Result<(Type, Expr)> {
        if let Some(header) = self.headers.get(name) {
            let args = self.arguments(args, Fields::Declared(&header.params), depth, name)?;
            return Ok((
                header.result.clone(),
                Expr::Call(CallTarget::Local(name.to_owned()), args),
            ));
        }
        if let Some((alias, name)) = name.split_once('.')
            && let Some(import) = self.resolver.imports.get(alias)
            && name == import.signature.name
        {
            let signature = &import.signature;
            let args = self.arguments(args, Fields::Declared(&signature.params), depth, name)?;
            return Ok((
                signature.result.clone(),
                Expr::Call(
                    CallTarget::Imported(alias.to_owned(), name.to_owned()),
                    args,
                ),
            ));
        }
        if let Some(variant) = self.variant(name, args, expected, depth)? {
            return Ok(variant);
        }
        self.builtin(name, args, expected, depth)
    }

    fn variant(
        &self,
        name: &str,
        args: SourceArgs<'_>,
        expected: Option<&Type>,
        depth: usize,
    ) -> Result<Option<(Type, Expr)>> {
        let Some((owner, variant)) = name.rsplit_once('.') else {
            return Ok(None);
        };
        let ty = match (owner, expected) {
            ("Maybe", Some(ty @ Type::Maybe { .. }))
            | ("Result", Some(ty @ Type::Result { .. })) => ty.clone(),
            ("Maybe", _) if variant == "some" => {
                let item = match args {
                    SourceArgs::Positional([item]) => item,
                    SourceArgs::Named([(label, item)]) if label == "value" => item,
                    _ => return Err(error("Maybe.some requires one value argument")),
                };
                let item = self.expression(item, None, depth)?;
                return Ok(Some((
                    Type::maybe(item.ty),
                    Expr::Variant(1, Arguments(vec![(0, item.expr)])),
                )));
            }
            ("Maybe", _) => return Err(error("Maybe.none requires an expected Maybe type")),
            ("Result", _) => {
                return Err(error("Result constructors require an expected Result type"));
            }
            _ => match self.resolver.nominal(owner, false) {
                Some(ty) => ty,
                None => return Ok(None),
            },
        };
        let union = ty
            .union(self.catalog)
            .ok_or_else(|| error(format!("{owner} is a record")))?;
        let (index, fields) = union
            .find(variant)
            .ok_or_else(|| error(format!("unknown variant {name}")))?;
        let args = self.arguments(args, fields, depth, name)?;
        Ok(Some((ty, Expr::Variant(index, args))))
    }

    fn arguments(
        &self,
        args: SourceArgs<'_>,
        fields: Fields<'_>,
        depth: usize,
        owner: &str,
    ) -> Result<Arguments> {
        arity(owner, fields.len(), args.len())?;
        let values = match args {
            SourceArgs::Positional(values) => values
                .iter()
                .zip(fields.iter())
                .enumerate()
                .map(|(index, (value, (_, ty)))| {
                    Ok((index, self.expression(value, Some(ty), depth)?.expr))
                })
                .collect::<Result<_>>()?,
            SourceArgs::Named(values) => {
                let by_name = fields
                    .iter()
                    .enumerate()
                    .map(|(index, (name, ty))| (name, (index, ty)))
                    .collect::<BTreeMap<_, _>>();
                let mut seen = BTreeSet::new();
                values
                    .iter()
                    .map(|(name, value)| {
                        let &(index, ty) = by_name.get(name.as_str()).ok_or_else(|| {
                            error(format!("{owner} has no argument or field {name}"))
                        })?;
                        if !seen.insert(index) {
                            return Err(error(format!("duplicate argument or field {name}")));
                        }
                        Ok((index, self.expression(value, Some(ty), depth)?.expr))
                    })
                    .collect::<Result<_>>()?
            }
        };
        Ok(Arguments(values))
    }

    fn builtin(
        &self,
        name: &str,
        args: SourceArgs<'_>,
        expected: Option<&Type>,
        depth: usize,
    ) -> Result<(Type, Expr)> {
        use Builtin::*;
        let (builtin, types, result): (Builtin, &[Type], Type) = match name {
            "bytes.len" => (BytesLen, &[Type::Bytes], Type::Int),
            "bytes.at" => (BytesAt, &[Type::Bytes, Type::Int], Type::Int),
            "bytes.slice" => (
                BytesSlice,
                &[Type::Bytes, Type::Int, Type::Int],
                Type::Bytes,
            ),
            "bytes.concat" => (BytesConcat, &[Type::Bytes, Type::Bytes], Type::Bytes),
            "bytes.from_int" => (Byte, &[Type::Int], Type::Bytes),
            "bytes.hex" => (BytesHex, &[Type::String], Type::Bytes),
            "bytes.encode_utf8" => (EncodeUtf8, &[Type::String], Type::Bytes),
            "bytes.decode_utf8" => (DecodeUtf8, &[Type::Bytes], Type::String),
            "string.len" => (StringLen, &[Type::String], Type::Int),
            "list.len" | "list.get" | "list.push" => {
                let (builtin, count) = match name {
                    "list.len" => (ListLen, 1),
                    "list.get" => (ListGet, 2),
                    _ => (ListPush, 2),
                };
                return self.list_builtin(
                    builtin,
                    name,
                    positional(name, args, count)?,
                    expected,
                    depth,
                );
            }
            _ => return Err(error(format!("unknown function or constructor {name}"))),
        };
        let args = positional(name, args, types.len())?
            .iter()
            .zip(types)
            .map(|(value, ty)| Ok(self.expression(value, Some(ty), depth)?.expr))
            .collect::<Result<_>>()?;
        Ok((result, Expr::Builtin(builtin, args)))
    }

    fn list_builtin(
        &self,
        builtin: Builtin,
        name: &str,
        args: &[ast::Expr],
        expected: Option<&Type>,
        depth: usize,
    ) -> Result<(Type, Expr)> {
        let list_type = match (builtin, expected) {
            (Builtin::ListGet, Some(item)) => Some(Type::list(item.clone())),
            (Builtin::ListPush, Some(ty @ Type::List { .. })) => Some(ty.clone()),
            _ => None,
        };
        let list = self.expression(&args[0], list_type.as_ref(), depth)?;
        let Type::List { item } = list.ty else {
            return Err(error(format!("{name} requires a List, got {}", list.ty)));
        };
        let (result, args) = match builtin {
            Builtin::ListLen => (Type::Int, vec![list.expr]),
            Builtin::ListGet => {
                let index = self.expression(&args[1], Some(&Type::Int), depth)?.expr;
                (*item, vec![list.expr, index])
            }
            Builtin::ListPush => {
                let value = self.expression(&args[1], Some(&item), depth)?.expr;
                (Type::List { item }, vec![list.expr, value])
            }
            _ => return Err(invariant()),
        };
        Ok((result, Expr::Builtin(builtin, args)))
    }
}

fn positional<'a>(owner: &str, args: SourceArgs<'a>, expected: usize) -> Result<&'a [ast::Expr]> {
    arity(owner, expected, args.len())?;
    match args {
        SourceArgs::Positional(args) => Ok(args),
        SourceArgs::Named(_) => Err(error(format!("{owner} requires positional arguments"))),
    }
}

fn arity(owner: &str, expected: usize, actual: usize) -> Result<()> {
    if expected == actual {
        Ok(())
    } else {
        Err(error(format!(
            "{owner} expects {expected} arguments, received {actual}"
        )))
    }
}

fn typed(ty: Type, expr: Expr, expected: Option<&Type>) -> Result<TypedExpr> {
    nesting(ty.depth() - 1)?;
    if let Some(expected) = expected
        && expected != &ty
    {
        return Err(error(format!("expected {expected}, got {ty}")));
    }
    Ok(TypedExpr { ty, expr })
}

fn nesting(depth: usize) -> Result<()> {
    if depth > MAX_NESTING {
        return Err(Error::new(
            "E_CORE1_LIMIT",
            "Core1 source nesting exceeds the limit",
        ));
    }
    Ok(())
}

fn error(message: impl Into<String>) -> Error {
    Error::new("E_CORE1_TYPE", message)
}
