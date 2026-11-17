use super::{
    value::{self, Budget, Value},
    *,
};

const MAX_DEPTH: usize = 128;

pub(super) fn evaluate(module: &CheckedModule, args: &Json, fuel: u64) -> Result<Json> {
    let params = &module.signature.params;
    let args = args
        .as_object()
        .ok_or_else(|| Error::new("E_CORE1_VALUE", "Core1 arguments must be a JSON object"))?;
    if args.len() != params.len() {
        return Err(Error::new(
            "E_CORE1_VALUE",
            "input must contain exactly the declared parameters",
        ));
    }

    let mut runtime = Runtime {
        budget: Budget::new(fuel),
    };
    runtime.budget.storage::<Value>(params.len())?;
    let mut decoded = Vec::with_capacity(params.len());
    for param in params {
        let arg = args.get(&param.name).ok_or_else(|| {
            Error::new("E_CORE1_VALUE", format!("missing parameter {}", param.name))
        })?;
        let value = value::decode(&param.ty, arg, &module.catalog, &mut runtime.budget, 0)
            .map_err(|error| error.at(param.name.clone()))?;
        decoded.push(value);
    }
    let result = runtime.function(module, &module.signature.name, decoded, 0)?;
    value::encode(
        &module.signature.result,
        &result,
        &module.catalog,
        &mut runtime.budget,
        0,
    )
}

struct Runtime {
    budget: Budget,
}

impl Runtime {
    fn function(
        &mut self,
        module: &CheckedModule,
        name: &str,
        arguments: Vec<Value>,
        depth: usize,
    ) -> Result<Value> {
        limit(depth)?;
        self.budget.step()?;
        self.budget.work(name.len())?;
        let function = module.functions.get(name).ok_or_else(invariant)?;
        self.budget.storage::<Option<Value>>(function.slots)?;
        self.budget.work(function.slots)?;
        let mut slots = vec![None; function.slots];
        if arguments.len() > slots.len() {
            return Err(invariant());
        }
        for (slot, argument) in slots.iter_mut().zip(arguments) {
            *slot = Some(argument);
        }
        let returned = self.block(module, &function.body, &mut slots, depth + 1)?;
        Ok(returned.unwrap_or(Value::Unit))
    }

    fn block(
        &mut self,
        module: &CheckedModule,
        block: &Block,
        slots: &mut [Option<Value>],
        depth: usize,
    ) -> Result<Option<Value>> {
        limit(depth)?;
        self.budget.work(block.slots.len())?;
        let mut returned = None;
        for statement in &block.statements {
            returned = self.statement(module, statement, slots, depth + 1)?;
            if returned.is_some() {
                break;
            }
        }
        for &slot in &block.slots {
            *slots.get_mut(slot).ok_or_else(invariant)? = None;
        }
        Ok(returned)
    }

    fn statement(
        &mut self,
        module: &CheckedModule,
        statement: &Stmt,
        slots: &mut [Option<Value>],
        depth: usize,
    ) -> Result<Option<Value>> {
        self.budget.step()?;
        match statement {
            Stmt::Store(slot, expression) => {
                let value = self.expression(module, expression, slots, depth)?;
                *slots.get_mut(*slot).ok_or_else(invariant)? = Some(value);
                Ok(None)
            }
            Stmt::Return(expression) => self.expression(module, expression, slots, depth).map(Some),
            Stmt::If(condition, yes, no) => {
                let result = self.expression(module, condition, slots, depth)?;
                match result {
                    Value::Bool(true) => self.block(module, yes, slots, depth),
                    Value::Bool(false) => self.block(module, no, slots, depth),
                    _ => Err(invariant()),
                }
            }
            Stmt::While(condition, body) => loop {
                match self.expression(module, condition, slots, depth)? {
                    Value::Bool(false) => return Ok(None),
                    Value::Bool(true) => {
                        if let Some(value) = self.block(module, body, slots, depth)? {
                            return Ok(Some(value));
                        }
                    }
                    _ => return Err(invariant()),
                }
            },
            Stmt::Match(expression, arms) => {
                let value = self.expression(module, expression, slots, depth)?;
                let Value::Variant(index, fields) = value else {
                    return Err(invariant());
                };
                for arm in arms {
                    self.budget.step()?;
                    if arm.variant.is_none() || arm.variant == Some(index) {
                        self.budget.work(arm.bindings.len())?;
                        for &(field, slot) in &arm.bindings {
                            let value = fields.items.get(field).ok_or_else(invariant)?;
                            *slots.get_mut(slot).ok_or_else(invariant)? = Some(value.clone());
                        }
                        return self.block(module, &arm.body, slots, depth);
                    }
                }
                Err(invariant())
            }
        }
    }

    fn expression(
        &mut self,
        module: &CheckedModule,
        expression: &Expr,
        slots: &[Option<Value>],
        depth: usize,
    ) -> Result<Value> {
        limit(depth)?;
        self.budget.step()?;
        let depth = depth + 1;
        match expression {
            Expr::Constant(value) => Ok(value.clone()),
            Expr::Local(slot) => slots
                .get(*slot)
                .and_then(Option::as_ref)
                .cloned()
                .ok_or_else(invariant),
            Expr::List(expressions) => {
                let values = self.values(module, expressions, slots, depth)?;
                Ok(Value::List(self.sequence(values)?))
            }
            Expr::Record(arguments) => {
                let values = self.arguments(module, arguments, slots, depth)?;
                Ok(Value::Record(self.sequence(values)?))
            }
            Expr::Variant(tag, arguments) => {
                let values = self.arguments(module, arguments, slots, depth)?;
                Ok(Value::Variant(*tag, self.sequence(values)?))
            }
            Expr::Member(base, index) => {
                let value = self.expression(module, base, slots, depth)?;
                let Value::Record(fields) = value else {
                    return Err(invariant());
                };
                fields.items.get(*index).cloned().ok_or_else(invariant)
            }
            Expr::Call(target, arguments) => {
                let values = self.arguments(module, arguments, slots, depth)?;
                match target {
                    CallTarget::Local(name) => self.function(module, name, values, depth),
                    CallTarget::Imported(alias, name) => {
                        self.budget.work(alias.len())?;
                        let import = module.imports.get(alias).ok_or_else(invariant)?;
                        self.function(import, name, values, depth)
                    }
                }
            }
            Expr::Builtin(builtin, arguments) => {
                let values = self.values(module, arguments, slots, depth)?;
                self.builtin(*builtin, &values)
            }
            Expr::Unary(operator, argument) => {
                let value = self.expression(module, argument, slots, depth)?;
                match (operator, value) {
                    (ast::Unary::Negate, Value::Int(value)) => integer(value.checked_neg()),
                    (ast::Unary::Not, Value::Bool(value)) => Ok(Value::Bool(!value)),
                    _ => Err(invariant()),
                }
            }
            Expr::Binary(operator, left, right) => {
                let left = self.expression(module, left, slots, depth)?;
                match (operator, &left) {
                    (ast::Binary::And, Value::Bool(false)) => return Ok(Value::Bool(false)),
                    (ast::Binary::Or, Value::Bool(true)) => return Ok(Value::Bool(true)),
                    _ => {}
                }
                let right = self.expression(module, right, slots, depth)?;
                self.binary(*operator, left, right)
            }
        }
    }

    fn values(
        &mut self,
        module: &CheckedModule,
        expressions: &[Expr],
        slots: &[Option<Value>],
        depth: usize,
    ) -> Result<Vec<Value>> {
        self.budget.storage::<Value>(expressions.len())?;
        let mut values = Vec::with_capacity(expressions.len());
        for expression in expressions {
            values.push(self.expression(module, expression, slots, depth)?);
        }
        Ok(values)
    }

    fn arguments(
        &mut self,
        module: &CheckedModule,
        arguments: &Arguments,
        slots: &[Option<Value>],
        depth: usize,
    ) -> Result<Vec<Value>> {
        self.budget.storage::<Value>(arguments.0.len())?;
        self.budget.work(arguments.0.len())?;
        let mut values = vec![Value::Unit; arguments.0.len()];
        for (index, expression) in &arguments.0 {
            let value = self.expression(module, expression, slots, depth)?;
            *values.get_mut(*index).ok_or_else(invariant)? = value;
        }
        Ok(values)
    }

    fn binary(&mut self, operator: ast::Binary, left: Value, right: Value) -> Result<Value> {
        use ast::Binary::*;
        match (operator, &left, &right) {
            (Equal | NotEqual, left, right) => {
                let equal = value::equal(left, right, &mut self.budget)?;
                Ok(Value::Bool(if matches!(operator, Equal) {
                    equal
                } else {
                    !equal
                }))
            }
            (Add, Value::String(left), Value::String(right)) => {
                let length = sum_lengths(left.len(), right.len())?;
                self.budget.storage::<u8>(length)?;
                self.budget.storage::<String>(1)?;
                self.budget.work(length)?;
                let mut joined = String::with_capacity(length);
                joined.push_str(left);
                joined.push_str(right);
                Ok(Value::String(Arc::new(joined)))
            }
            (Add, Value::Int(a), Value::Int(b)) => integer(a.checked_add(*b)),
            (Subtract, Value::Int(a), Value::Int(b)) => integer(a.checked_sub(*b)),
            (Multiply, Value::Int(a), Value::Int(b)) => integer(a.checked_mul(*b)),
            (Divide | Remainder, Value::Int(_), Value::Int(0)) => {
                Err(Error::new("E_CORE1_DIVIDE_BY_ZERO", "Int division by zero"))
            }
            (Divide, Value::Int(a), Value::Int(b)) => integer(a.checked_div(*b)),
            (Remainder, Value::Int(a), Value::Int(b)) => integer(a.checked_rem(*b)),
            (Less, Value::Int(a), Value::Int(b)) => Ok(Value::Bool(a < b)),
            (LessEqual, Value::Int(a), Value::Int(b)) => Ok(Value::Bool(a <= b)),
            (Greater, Value::Int(a), Value::Int(b)) => Ok(Value::Bool(a > b)),
            (GreaterEqual, Value::Int(a), Value::Int(b)) => Ok(Value::Bool(a >= b)),
            (And, Value::Bool(a), Value::Bool(b)) => Ok(Value::Bool(*a && *b)),
            (Or, Value::Bool(a), Value::Bool(b)) => Ok(Value::Bool(*a || *b)),
            _ => Err(invariant()),
        }
    }

    fn builtin(&mut self, builtin: Builtin, args: &[Value]) -> Result<Value> {
        use Builtin::*;
        match (builtin, args) {
            (BytesLen, [Value::Bytes(bytes)]) => length(bytes.len()),
            (BytesAt, [Value::Bytes(bytes), Value::Int(at)]) => bytes
                .get(index(*at)?)
                .map(|byte| Value::Int(i64::from(*byte)))
                .ok_or_else(bounds),
            (BytesSlice, [Value::Bytes(bytes), Value::Int(start), Value::Int(end)]) => {
                self.copy_bytes(bytes.get(index(*start)?..index(*end)?).ok_or_else(bounds)?)
            }
            (BytesConcat, [Value::Bytes(left), Value::Bytes(right)]) => {
                let length = sum_lengths(left.len(), right.len())?;
                self.budget.storage::<u8>(length)?;
                self.budget.storage::<Vec<u8>>(1)?;
                self.budget.work(length)?;
                let mut joined = Vec::with_capacity(length);
                joined.extend_from_slice(left);
                joined.extend_from_slice(right);
                Ok(Value::Bytes(Arc::new(joined)))
            }
            (Byte, [Value::Int(byte)]) => {
                let byte = u8::try_from(*byte).map_err(|_| bounds())?;
                self.copy_bytes(&[byte])
            }
            (BytesHex, [Value::String(hex)]) => {
                self.budget.storage::<Vec<u8>>(1)?;
                let bytes = value::from_hex(hex, &mut self.budget)?;
                Ok(Value::Bytes(Arc::new(bytes)))
            }
            (EncodeUtf8, [Value::String(text)]) => self.copy_bytes(text.as_bytes()),
            (DecodeUtf8, [Value::Bytes(bytes)]) => {
                self.budget.work(bytes.len())?;
                let text = std::str::from_utf8(bytes)
                    .map_err(|_| Error::new("E_CORE1_UTF8", "Bytes are not valid UTF-8"))?;
                self.budget.storage::<u8>(text.len())?;
                self.budget.storage::<String>(1)?;
                self.budget.work(text.len())?;
                Ok(Value::String(Arc::new(text.to_owned())))
            }
            (StringLen, [Value::String(text)]) => {
                self.budget.work(text.len())?;
                length(text.chars().count())
            }
            (ListLen, [Value::List(items)]) => length(items.items.len()),
            (ListGet, [Value::List(items), Value::Int(at)]) => {
                items.items.get(index(*at)?).cloned().ok_or_else(bounds)
            }
            (ListPush, [Value::List(list), item]) => {
                let length = sum_lengths(list.items.len(), 1)?;
                self.budget.storage::<Value>(length)?;
                self.budget.work(length)?;
                let mut items = Vec::with_capacity(length);
                items.extend_from_slice(&list.items);
                items.push(item.clone());
                Ok(Value::List(self.sequence(items)?))
            }
            _ => Err(invariant()),
        }
    }

    fn copy_bytes(&mut self, bytes: &[u8]) -> Result<Value> {
        self.budget.storage::<u8>(bytes.len())?;
        self.budget.storage::<Vec<u8>>(1)?;
        self.budget.work(bytes.len())?;
        Ok(Value::Bytes(Arc::new(bytes.to_vec())))
    }

    fn sequence(&mut self, values: Vec<Value>) -> Result<Arc<value::Sequence>> {
        self.budget.storage::<value::Sequence>(1)?;
        self.budget.work(values.len())?;
        value::sequence(values)
    }
}

fn limit(depth: usize) -> Result<()> {
    if depth > MAX_DEPTH {
        return Err(Error::new(
            "E_CORE1_LIMIT",
            "Core1 execution nesting exceeds the limit",
        ));
    }
    Ok(())
}

fn integer(value: Option<i64>) -> Result<Value> {
    value
        .map(Value::Int)
        .ok_or_else(|| Error::new("E_CORE1_OVERFLOW", "checked Int overflow"))
}

fn length(amount: usize) -> Result<Value> {
    integer(i64::try_from(amount).ok())
}

fn sum_lengths(left: usize, right: usize) -> Result<usize> {
    left.checked_add(right)
        .ok_or_else(|| Error::new("E_CORE1_LIMIT", "Core1 value exceeds the size limit"))
}

fn index(index: i64) -> Result<usize> {
    usize::try_from(index).map_err(|_| bounds())
}

fn bounds() -> Error {
    Error::new("E_CORE1_BOUNDS", "index, byte or slice is out of range")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core1::parser;

    fn module(source: &str) -> CheckedModule {
        let ast = parser::parse("engine-test", source).unwrap();
        check(ast, BTreeMap::new(), Digest::of_bytes(source.as_bytes())).unwrap()
    }

    #[test]
    fn recursive_helpers_consume_execution_depth() {
        let module = module(
            "nexa 0.3\nmodule tests.recursion\n\
             fn descend() -> Int { return descend() }\n\
             export fn begin() -> Int { return descend() }\n",
        );
        let error = module
            .evaluate(&serde_json::json!({}), 100_000)
            .unwrap_err();
        assert_eq!(error.diagnostic.code, "E_CORE1_LIMIT");
    }

    #[test]
    fn copying_bytes_consumes_fuel_proportional_to_size() {
        let module = module(
            r#"nexa 0.3
module tests.byte_copy
export fn grow(seed is Bytes) -> Int {
    var buffer = seed
    var count = 0
    while count < 18 {
        buffer = bytes.concat(buffer, buffer)
        count = count + 1
    }
    return bytes.len(buffer)
}
"#,
        );
        let args = serde_json::json!({"seed": "00"});
        let error = module.evaluate(&args, 10_000).unwrap_err();
        assert_eq!(error.diagnostic.code, "E_CORE1_FUEL");
        let returned = module.evaluate(&args, 1_000_000).unwrap();
        assert_eq!(returned, "262144");
    }
}
