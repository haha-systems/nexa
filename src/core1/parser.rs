use super::{
    SOURCE_LIMIT,
    ast::{
        Arm, Binary, Expr, Field, Function, Module, Stmt, TypeDecl, TypeDeclKind, TypeExpr, Unary,
        Variant,
    },
};
use crate::diagnostic::{Error, Result, SourceSpan};

const MAX_TOKENS: usize = 64 * 1024;
const MAX_SYNTAX_NESTING: usize = 64;
const MAX_EXPRESSION_DEPTH: usize = 96;

pub fn parse(source_id: &str, text: &str) -> Result<Module> {
    if text.len() > SOURCE_LIMIT {
        return Err(spanned_error(
            source_id,
            0,
            text.len(),
            "E_SOURCE_LIMIT",
            format!("Core1 sources may contain at most {SOURCE_LIMIT} bytes"),
        ));
    }

    Parser {
        source_id,
        text,
        tokens: Lexer {
            source_id,
            text,
            position: 0,
        }
        .lex()?,
        position: 0,
    }
    .module()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Symbol {
    OpenParen,
    CloseParen,
    OpenBrace,
    CloseBrace,
    OpenBracket,
    CloseBracket,
    Dot,
    Comma,
    Colon,
    Semicolon,
    Pipe,
    Arrow,
    ArmArrow,
    Assign,
    Equal,
    NotEqual,
    Less,
    LessEqual,
    Greater,
    GreaterEqual,
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
}

#[derive(Debug, PartialEq, Eq)]
enum TokenKind {
    Word,
    Number,
    String(String),
    Bytes(Vec<u8>),
    Symbol(Symbol),
    End,
}

#[derive(Debug)]
struct Token {
    kind: TokenKind,
    start: usize,
    end: usize,
    leading_newline: bool,
}

struct Lexer<'a> {
    source_id: &'a str,
    text: &'a str,
    position: usize,
}

impl Lexer<'_> {
    fn lex(mut self) -> Result<Vec<Token>> {
        let mut tokens = Vec::new();
        let mut leading_newline = false;
        while let Some(ch) = self.peek() {
            if ch.is_whitespace() {
                leading_newline |= is_newline(ch);
                self.advance();
                continue;
            }

            let start = self.position;
            if self.consume("//") {
                self.advance_while(|ch| !is_newline(ch));
                continue;
            }
            if self.consume("/*") {
                loop {
                    if self.consume("*/") {
                        break;
                    }
                    let ch = self.advance().ok_or_else(|| {
                        self.error(start, "E_COMMENT", "unterminated block comment")
                    })?;
                    leading_newline |= is_newline(ch);
                }
                continue;
            }

            if tokens.len() == MAX_TOKENS {
                return Err(self.error(
                    start,
                    "E_SOURCE_LIMIT",
                    format!("Core1 sources may contain at most {MAX_TOKENS} tokens"),
                ));
            }

            let kind = if self.consume("hex\"") {
                TokenKind::Bytes(self.byte_literal(start)?)
            } else {
                match ch {
                    'A'..='Z' | 'a'..='z' | '_' => {
                        self.advance_while(|next| next.is_ascii_alphanumeric() || next == '_');
                        TokenKind::Word
                    }
                    '0'..='9' => {
                        self.number();
                        TokenKind::Number
                    }
                    '"' => TokenKind::String(self.string_literal(start)?),
                    _ => TokenKind::Symbol(self.symbol(start)?),
                }
            };
            tokens.push(Token {
                kind,
                start,
                end: self.position,
                leading_newline: std::mem::take(&mut leading_newline),
            });
        }
        tokens.push(Token {
            kind: TokenKind::End,
            start: self.position,
            end: self.position,
            leading_newline,
        });
        Ok(tokens)
    }

    fn number(&mut self) {
        self.advance_while(|ch| ch.is_ascii_digit());
        // Keep a dotted version together so the header can require exactly 0.3.
        while let Some(after_dot) = self.rest().strip_prefix('.') {
            if !after_dot.starts_with(|ch: char| ch.is_ascii_digit()) {
                break;
            }
            self.position += 1;
            self.advance_while(|ch| ch.is_ascii_digit());
        }
    }

    fn symbol(&mut self, start: usize) -> Result<Symbol> {
        let ch = self.advance().expect("lexer is at a symbol");
        let symbol = match ch {
            '(' => Symbol::OpenParen,
            ')' => Symbol::CloseParen,
            '{' => Symbol::OpenBrace,
            '}' => Symbol::CloseBrace,
            '[' => Symbol::OpenBracket,
            ']' => Symbol::CloseBracket,
            '.' => Symbol::Dot,
            ',' => Symbol::Comma,
            ':' => Symbol::Colon,
            ';' => Symbol::Semicolon,
            '|' => Symbol::Pipe,
            '+' => Symbol::Plus,
            '-' if self.consume(">") => Symbol::Arrow,
            '-' => Symbol::Minus,
            '*' => Symbol::Star,
            '/' => Symbol::Slash,
            '%' => Symbol::Percent,
            '=' if self.consume("=") => Symbol::Equal,
            '=' if self.consume(">") => Symbol::ArmArrow,
            '=' => Symbol::Assign,
            '!' if self.consume("=") => Symbol::NotEqual,
            '<' if self.consume("=") => Symbol::LessEqual,
            '<' => Symbol::Less,
            '>' if self.consume("=") => Symbol::GreaterEqual,
            '>' => Symbol::Greater,
            _ => {
                return Err(self.error(
                    start,
                    "E_LEX",
                    format!("unsupported Core1 character {ch:?}"),
                ));
            }
        };
        Ok(symbol)
    }

    fn byte_literal(&mut self, start: usize) -> Result<Vec<u8>> {
        let digits_start = self.position;
        self.advance_while(|ch| ch.is_ascii_hexdigit());
        let digits = &self.text[digits_start..self.position];
        if !self.consume("\"") || !digits.len().is_multiple_of(2) {
            return Err(self.error(
                start,
                "E_BYTES",
                "byte literals require a closing quote and pairs of hexadecimal digits",
            ));
        }
        let digit = |byte| {
            char::from(byte)
                .to_digit(16)
                .expect("the lexer collected only hexadecimal digits") as u8
        };
        let (pairs, _) = digits.as_bytes().as_chunks::<2>();
        Ok(pairs
            .iter()
            .map(|&[high, low]| digit(high) * 16 + digit(low))
            .collect())
    }

    fn string_literal(&mut self, start: usize) -> Result<String> {
        let quote = if self.consume("\"\"\"") {
            "\"\"\""
        } else {
            self.advance();
            "\""
        };
        let mut value = String::new();
        while !self.consume(quote) {
            let char_start = self.position;
            let ch = self
                .advance()
                .ok_or_else(|| self.error(start, "E_STRING", "unterminated string literal"))?;
            if quote == "\"" && is_newline(ch) {
                return Err(self.error(
                    start,
                    "E_STRING",
                    "newline in a quoted string; use a triple-quoted string",
                ));
            }
            value.push(if ch == '\\' {
                self.string_escape(char_start)?
            } else {
                ch
            });
        }
        Ok(value)
    }

    fn string_escape(&mut self, start: usize) -> Result<char> {
        match self.advance() {
            Some('"') => Ok('"'),
            Some('\\') => Ok('\\'),
            Some('n') => Ok('\n'),
            Some('r') => Ok('\r'),
            Some('t') => Ok('\t'),
            Some('u') if self.consume("{") => {
                let digits_start = self.position;
                self.advance_while(|ch| ch.is_ascii_hexdigit());
                let digits = &self.text[digits_start..self.position];
                if self.consume("}")
                    && let Some(ch) = u32::from_str_radix(digits, 16)
                        .ok()
                        .and_then(char::from_u32)
                {
                    return Ok(ch);
                }
                Err(self.error(start, "E_STRING_ESCAPE", "invalid Unicode scalar escape"))
            }
            Some('u') => Err(self.error(
                start,
                "E_STRING_ESCAPE",
                "Unicode escapes must use \\u{HEX}",
            )),
            Some(ch) => Err(self.error(
                start,
                "E_STRING_ESCAPE",
                format!("unsupported string escape \\{ch}"),
            )),
            None => Err(self.error(start, "E_STRING_ESCAPE", "incomplete string escape")),
        }
    }

    fn rest(&self) -> &str {
        &self.text[self.position..]
    }

    fn peek(&self) -> Option<char> {
        self.rest().chars().next()
    }

    fn advance(&mut self) -> Option<char> {
        let ch = self.peek()?;
        self.position += ch.len_utf8();
        Some(ch)
    }

    fn advance_while(&mut self, predicate: impl Fn(char) -> bool) {
        while self.peek().is_some_and(&predicate) {
            self.advance();
        }
    }

    fn consume(&mut self, prefix: &str) -> bool {
        if self.rest().starts_with(prefix) {
            self.position += prefix.len();
            true
        } else {
            false
        }
    }

    fn error(&self, start: usize, code: &str, message: impl Into<String>) -> Error {
        spanned_error(self.source_id, start, self.position, code, message)
    }
}

fn is_newline(ch: char) -> bool {
    matches!(ch, '\n' | '\r')
}

#[derive(Clone, Copy)]
enum ExprContext {
    Statement,
    Delimited,
    BeforeBlock,
}

struct ParsedExpr {
    value: Expr,
    depth: usize,
}

impl ParsedExpr {
    fn leaf(value: Expr) -> Self {
        Self { value, depth: 0 }
    }

    fn path(&self) -> Option<String> {
        let mut value = &self.value;
        let mut segments = Vec::new();
        while let Expr::Member(base, name) = value {
            segments.push(name.as_str());
            value = base;
        }
        let Expr::Name(name) = value else {
            return None;
        };
        segments.push(name);
        segments.reverse();
        Some(segments.join("."))
    }
}

struct Parser<'a> {
    source_id: &'a str,
    text: &'a str,
    tokens: Vec<Token>,
    position: usize,
}

impl Parser<'_> {
    fn module(mut self) -> Result<Module> {
        self.expect_word("nexa")?;
        if self.current().kind != TokenKind::Number || self.lexeme() != "0.3" {
            return Err(self.error("E_VERSION", "expected language version 0.3"));
        }
        self.advance();
        self.separator("language and module headers")?;
        self.expect_word("module")?;
        let name = self.path(false, "module name")?;
        let mut module = Module {
            name,
            imports: Vec::new(),
            types: Vec::new(),
            functions: Vec::new(),
        };

        while !self.is_end() {
            self.separator("module header and declarations")?;
            if self.is_end() {
                break;
            }
            if self.take_word("import") {
                module.imports.push(self.identifier("import alias")?);
                continue;
            }

            let exported = self.take_word("export");
            if self.take_word("type") {
                module.types.push(self.type_declaration(exported)?);
            } else if self.take_word("fn") {
                module.functions.push(self.function(exported)?);
            } else {
                return Err(self.error("E_UNSUPPORTED", "expected a Core1 type or fn declaration"));
            }
        }
        Ok(module)
    }

    fn type_declaration(&mut self, exported: bool) -> Result<TypeDecl> {
        let name = self.identifier("type name")?;
        self.expect_word("is")?;
        let kind = if self.take(Symbol::OpenBrace) {
            TypeDeclKind::Record(self.items(Symbol::CloseBrace, "}", true, |parser| {
                parser.field("record field")
            })?)
        } else {
            let mut variants = Vec::new();
            loop {
                let name = self.identifier("variant name")?;
                let fields = if self.take_on_line(Symbol::OpenParen) {
                    self.items(Symbol::CloseParen, ")", false, |parser| {
                        parser.field("variant field")
                    })?
                } else {
                    Vec::new()
                };
                variants.push(Variant { name, fields });
                if !self.take(Symbol::Pipe) {
                    break;
                }
            }
            TypeDeclKind::Union(variants)
        };
        Ok(TypeDecl {
            name,
            exported,
            kind,
        })
    }

    fn function(&mut self, exported: bool) -> Result<Function> {
        let name = self.identifier("function name")?;
        self.expect(Symbol::OpenParen, "(")?;
        let params = self.items(Symbol::CloseParen, ")", false, |parser| {
            parser.field("parameter name")
        })?;
        self.expect(Symbol::Arrow, "->")?;
        let result = self.type_expression(0)?;
        let body = self.block(0)?;
        Ok(Function {
            name,
            exported,
            params,
            result,
            body,
        })
    }

    fn field(&mut self, context: &str) -> Result<Field> {
        let name = self.identifier(context)?;
        self.expect_word("is")?;
        let ty = self.type_expression(0)?;
        Ok(Field { name, ty })
    }

    fn type_expression(&mut self, nesting: usize) -> Result<TypeExpr> {
        self.check_nesting(nesting)?;
        let name = self.path(false, "type name")?;
        if self.take_on_line(Symbol::OpenBracket) {
            let args = self.items(Symbol::CloseBracket, "]", false, |parser| {
                parser.type_expression(nesting + 1)
            })?;
            if args.is_empty() {
                return Err(self.error("E_PARSE", "expected at least one type argument"));
            }
            Ok(TypeExpr::Generic(name, args))
        } else {
            Ok(TypeExpr::Named(name))
        }
    }

    fn block(&mut self, nesting: usize) -> Result<Vec<Stmt>> {
        self.check_nesting(nesting)?;
        self.expect(Symbol::OpenBrace, "{")?;
        let mut statements = Vec::new();
        while !self.take(Symbol::CloseBrace) {
            if self.is_end() {
                return Err(self.error("E_PARSE", "expected }"));
            }
            statements.push(self.statement(nesting)?);
            if !self.at(Symbol::CloseBrace) {
                self.separator("statements")?;
            }
        }
        Ok(statements)
    }

    fn statement(&mut self, nesting: usize) -> Result<Stmt> {
        let mutable = self.take_word("var");
        if mutable || self.take_word("let") {
            let name = self.identifier("local name")?;
            let ty = if self.take_word("is") {
                Some(self.type_expression(0)?)
            } else {
                None
            };
            self.expect(Symbol::Assign, "=")?;
            let value = self.expression(ExprContext::Statement)?;
            return Ok(Stmt::Bind {
                name,
                mutable,
                ty,
                value,
            });
        }
        if self.take_word("return") {
            return Ok(Stmt::Return(self.expression(ExprContext::Statement)?));
        }
        if self.take_word("if") {
            return self.if_statement(nesting);
        }
        if self.take_word("while") {
            let condition = self.expression(ExprContext::BeforeBlock)?;
            let body = self.block(nesting + 1)?;
            return Ok(Stmt::While { condition, body });
        }
        if self.take_word("match") {
            return self.match_statement(nesting);
        }
        if self.current().kind == TokenKind::Word
            && self.token(1).kind == TokenKind::Symbol(Symbol::Assign)
            && !self.token(1).leading_newline
        {
            let name = self.identifier("local name")?;
            self.advance();
            let value = self.expression(ExprContext::Statement)?;
            return Ok(Stmt::Assign { name, value });
        }
        Err(self.error("E_UNSUPPORTED", "expected a Core1 statement"))
    }

    fn if_statement(&mut self, nesting: usize) -> Result<Stmt> {
        self.check_nesting(nesting)?;
        let condition = self.expression(ExprContext::BeforeBlock)?;
        let then_body = self.block(nesting + 1)?;
        let else_body = if self.take_word("else") {
            if self.take_word("if") {
                vec![self.if_statement(nesting + 1)?]
            } else {
                self.block(nesting + 1)?
            }
        } else {
            Vec::new()
        };
        Ok(Stmt::If {
            condition,
            then_body,
            else_body,
        })
    }

    fn match_statement(&mut self, nesting: usize) -> Result<Stmt> {
        let value = self.expression(ExprContext::BeforeBlock)?;
        self.expect(Symbol::OpenBrace, "{")?;
        let mut arms = Vec::new();
        while !self.take(Symbol::CloseBrace) {
            if self.is_end() {
                return Err(self.error("E_PARSE", "expected }"));
            }
            let variant = self.path(true, "match variant")?;
            let bindings = if self.take_on_line(Symbol::OpenParen) {
                self.items(Symbol::CloseParen, ")", false, |parser| {
                    let field = parser.identifier("variant field")?;
                    parser.expect(Symbol::Colon, ":")?;
                    let name = parser.identifier("pattern binding")?;
                    Ok((field, name))
                })?
            } else {
                Vec::new()
            };
            self.expect(Symbol::ArmArrow, "=>")?;
            let body = self.block(nesting + 1)?;
            arms.push(Arm {
                variant,
                bindings,
                body,
            });
            if !self.at(Symbol::CloseBrace) {
                self.separator("match arms")?;
            }
        }
        Ok(Stmt::Match { value, arms })
    }

    fn expression(&mut self, context: ExprContext) -> Result<Expr> {
        Ok(self.binary_expression(0, context, 0)?.value)
    }

    fn binary_expression(
        &mut self,
        minimum: u8,
        context: ExprContext,
        nesting: usize,
    ) -> Result<ParsedExpr> {
        const COMPARISON: u8 = 3;

        self.check_nesting(nesting)?;
        let mut left = self.prefix_expression(context, nesting)?;
        let mut compared = false;
        while let Some((precedence, operator)) = self.binary_operator() {
            if precedence < minimum
                || self.current().leading_newline && !matches!(context, ExprContext::Delimited)
            {
                break;
            }
            if precedence == COMPARISON {
                if compared {
                    return Err(self.error("E_PARSE", "comparison operators cannot be chained"));
                }
                compared = true;
            }
            self.advance();
            let right = self.binary_expression(precedence + 1, context, nesting + 1)?;
            left = self.expression_node(
                Expr::Binary(operator, Box::new(left.value), Box::new(right.value)),
                left.depth.max(right.depth),
            )?;
        }
        Ok(left)
    }

    fn binary_operator(&self) -> Option<(u8, Binary)> {
        let operator = match self.current().kind {
            TokenKind::Word if self.lexeme() == "or" => (1, Binary::Or),
            TokenKind::Word if self.lexeme() == "and" => (2, Binary::And),
            TokenKind::Symbol(Symbol::Equal) => (3, Binary::Equal),
            TokenKind::Symbol(Symbol::NotEqual) => (3, Binary::NotEqual),
            TokenKind::Symbol(Symbol::Less) => (3, Binary::Less),
            TokenKind::Symbol(Symbol::LessEqual) => (3, Binary::LessEqual),
            TokenKind::Symbol(Symbol::Greater) => (3, Binary::Greater),
            TokenKind::Symbol(Symbol::GreaterEqual) => (3, Binary::GreaterEqual),
            TokenKind::Symbol(Symbol::Plus) => (4, Binary::Add),
            TokenKind::Symbol(Symbol::Minus) => (4, Binary::Subtract),
            TokenKind::Symbol(Symbol::Star) => (5, Binary::Multiply),
            TokenKind::Symbol(Symbol::Slash) => (5, Binary::Divide),
            TokenKind::Symbol(Symbol::Percent) => (5, Binary::Remainder),
            _ => return None,
        };
        Some(operator)
    }

    fn prefix_expression(&mut self, context: ExprContext, nesting: usize) -> Result<ParsedExpr> {
        self.check_nesting(nesting)?;
        let operator = if self.take_word("not") {
            Unary::Not
        } else if self.take(Symbol::Minus) {
            if self.current().kind == TokenKind::Number
                && self.lexeme().parse::<u64>().ok() == Some(i64::MIN.unsigned_abs())
            {
                self.advance();
                return self.postfix_expression(
                    ParsedExpr::leaf(Expr::Int(i64::MIN)),
                    context,
                    nesting,
                );
            }
            Unary::Negate
        } else {
            let atom = self.atom(nesting)?;
            return self.postfix_expression(atom, context, nesting);
        };

        let operand = self.prefix_expression(context, nesting + 1)?;
        self.expression_node(
            Expr::Unary(operator, Box::new(operand.value)),
            operand.depth,
        )
    }

    fn atom(&mut self, nesting: usize) -> Result<ParsedExpr> {
        let value =
            match &self.current().kind {
                TokenKind::String(value) => Expr::String(value.clone()),
                TokenKind::Bytes(value) => Expr::Bytes(value.clone()),
                TokenKind::Number => Expr::Int(self.lexeme().parse::<i64>().map_err(|_| {
                    self.error("E_INT", "expected a signed 64-bit integer literal")
                })?),
                TokenKind::Word => match self.lexeme() {
                    "unit" => Expr::Unit,
                    "true" => Expr::Bool(true),
                    "false" => Expr::Bool(false),
                    "none" => Expr::Call("Maybe.none".into(), Vec::new()),
                    _ => return Ok(ParsedExpr::leaf(Expr::Name(self.identifier("expression")?))),
                },
                TokenKind::Symbol(Symbol::OpenParen) => {
                    self.advance();
                    let inner = self.binary_expression(0, ExprContext::Delimited, nesting + 1)?;
                    self.expect(Symbol::CloseParen, ")")?;
                    return Ok(inner);
                }
                TokenKind::Symbol(Symbol::OpenBracket) => {
                    self.advance();
                    let (elements, depth) =
                        self.positional_values(Symbol::CloseBracket, "]", nesting)?;
                    return self.expression_node(Expr::List(elements), depth);
                }
                _ => return Err(self.error("E_PARSE", "expected a Core1 expression")),
            };
        self.advance();
        Ok(ParsedExpr::leaf(value))
    }

    fn postfix_expression(
        &mut self,
        mut expression: ParsedExpr,
        context: ExprContext,
        nesting: usize,
    ) -> Result<ParsedExpr> {
        while !self.current().leading_newline {
            if self.take(Symbol::Dot) {
                let name = self.variant_segment("member name")?;
                expression = self.expression_node(
                    Expr::Member(Box::new(expression.value), name),
                    expression.depth,
                )?;
            } else if self.take(Symbol::OpenParen) {
                let path = expression.path().ok_or_else(|| {
                    self.error("E_PARSE", "calls require a name or qualified path")
                })?;
                expression = self.call(path, nesting)?;
            } else if self.record_ahead(context)
                && let Some(path) = expression.path()
            {
                self.advance();
                let (fields, depth) = self.labeled_values(Symbol::CloseBrace, "}", nesting)?;
                expression = self.expression_node(Expr::Record(path, fields), depth)?;
            } else {
                break;
            }
        }
        Ok(expression)
    }

    fn record_ahead(&self, context: ExprContext) -> bool {
        self.at(Symbol::OpenBrace)
            && ((self.token(1).kind == TokenKind::Word
                && self.token(2).kind == TokenKind::Symbol(Symbol::Colon))
                || (!matches!(context, ExprContext::BeforeBlock)
                    && self.token(1).kind == TokenKind::Symbol(Symbol::CloseBrace)))
    }

    fn call(&mut self, path: String, nesting: usize) -> Result<ParsedExpr> {
        if self.current().kind == TokenKind::Word
            && self.token(1).kind == TokenKind::Symbol(Symbol::Colon)
        {
            let (args, depth) = self.labeled_values(Symbol::CloseParen, ")", nesting)?;
            self.expression_node(Expr::NamedCall(path, args), depth)
        } else {
            let (args, depth) = self.positional_values(Symbol::CloseParen, ")", nesting)?;
            self.expression_node(Expr::Call(path, args), depth)
        }
    }

    fn positional_values(
        &mut self,
        closing: Symbol,
        closing_text: &str,
        nesting: usize,
    ) -> Result<(Vec<Expr>, usize)> {
        let mut depth = 0;
        let values = self.items(closing, closing_text, false, |parser| {
            let expression = parser.binary_expression(0, ExprContext::Delimited, nesting + 1)?;
            depth = depth.max(expression.depth);
            Ok(expression.value)
        })?;
        Ok((values, depth))
    }

    fn labeled_values(
        &mut self,
        closing: Symbol,
        closing_text: &str,
        nesting: usize,
    ) -> Result<(Vec<(String, Expr)>, usize)> {
        let is_record = closing == Symbol::CloseBrace;
        let context = if is_record {
            ExprContext::Statement
        } else {
            ExprContext::Delimited
        };
        let mut depth = 0;
        let values = self.items(closing, closing_text, is_record, |parser| {
            let name = parser.identifier("field or argument name")?;
            parser.expect(Symbol::Colon, ":")?;
            let expression = parser.binary_expression(0, context, nesting + 1)?;
            depth = depth.max(expression.depth);
            Ok((name, expression.value))
        })?;
        Ok((values, depth))
    }

    fn items<T>(
        &mut self,
        closing: Symbol,
        closing_text: &str,
        newlines_separate: bool,
        mut item: impl FnMut(&mut Self) -> Result<T>,
    ) -> Result<Vec<T>> {
        let mut items = Vec::new();
        while !self.take(closing) {
            if self.is_end() {
                return Err(self.error("E_PARSE", format!("expected {closing_text}")));
            }
            items.push(item(self)?);
            if !self.at(closing)
                && !self.take(Symbol::Comma)
                && !(newlines_separate && self.current().leading_newline)
            {
                return Err(self.error("E_SEPARATOR", format!("expected , or {closing_text}")));
            }
        }
        Ok(items)
    }

    fn expression_node(&self, value: Expr, child_depth: usize) -> Result<ParsedExpr> {
        if child_depth >= MAX_EXPRESSION_DEPTH {
            return Err(self.error(
                "E_NESTING_LIMIT",
                format!("Core1 expression depth exceeds {MAX_EXPRESSION_DEPTH}"),
            ));
        }
        Ok(ParsedExpr {
            value,
            depth: child_depth + 1,
        })
    }

    fn check_nesting(&self, nesting: usize) -> Result<()> {
        if nesting > MAX_SYNTAX_NESTING {
            return Err(self.error(
                "E_NESTING_LIMIT",
                format!("Core1 syntax nesting exceeds {MAX_SYNTAX_NESTING}"),
            ));
        }
        Ok(())
    }

    fn separator(&mut self, context: &str) -> Result<()> {
        if self.current().leading_newline || self.at(Symbol::Semicolon) {
            while self.take(Symbol::Semicolon) {}
            Ok(())
        } else {
            Err(self.error(
                "E_SEPARATOR",
                format!("{context} must be separated by a newline or semicolon"),
            ))
        }
    }

    fn path(&mut self, variant: bool, context: &str) -> Result<String> {
        let mut path = if variant {
            self.variant_segment(context)?
        } else {
            self.identifier(context)?
        };
        while self.take_on_line(Symbol::Dot) {
            let segment = if variant {
                self.variant_segment("path segment")?
            } else {
                self.identifier("path segment")?
            };
            path.push('.');
            path.push_str(&segment);
        }
        Ok(path)
    }

    fn variant_segment(&mut self, context: &str) -> Result<String> {
        if self.take_word("none") {
            Ok("none".into())
        } else {
            self.identifier(context)
        }
    }

    fn identifier(&mut self, context: &str) -> Result<String> {
        if self.current().kind != TokenKind::Word {
            return Err(self.error("E_PARSE", format!("expected {context}")));
        }
        let name = self.lexeme();
        if is_keyword(name) {
            return Err(self.error(
                "E_IDENTIFIER",
                format!("reserved word {name} cannot be used as {context}"),
            ));
        }
        let name = name.to_owned();
        self.advance();
        Ok(name)
    }

    fn expect_word(&mut self, word: &str) -> Result<()> {
        if self.take_word(word) {
            Ok(())
        } else {
            Err(self.error("E_PARSE", format!("expected {word}")))
        }
    }

    fn take_word(&mut self, word: &str) -> bool {
        if self.current().kind == TokenKind::Word && self.lexeme() == word {
            self.advance();
            true
        } else {
            false
        }
    }

    fn expect(&mut self, symbol: Symbol, text: &str) -> Result<()> {
        if self.take(symbol) {
            Ok(())
        } else {
            Err(self.error("E_PARSE", format!("expected {text}")))
        }
    }

    fn take(&mut self, symbol: Symbol) -> bool {
        if self.at(symbol) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn take_on_line(&mut self, symbol: Symbol) -> bool {
        !self.current().leading_newline && self.take(symbol)
    }

    fn at(&self, symbol: Symbol) -> bool {
        self.current().kind == TokenKind::Symbol(symbol)
    }

    fn is_end(&self) -> bool {
        self.current().kind == TokenKind::End
    }

    fn current(&self) -> &Token {
        self.token(0)
    }

    fn token(&self, offset: usize) -> &Token {
        &self.tokens[(self.position + offset).min(self.tokens.len() - 1)]
    }

    fn lexeme(&self) -> &str {
        let token = self.current();
        &self.text[token.start..token.end]
    }

    fn advance(&mut self) {
        self.position += 1;
    }

    fn error(&self, code: &str, message: impl Into<String>) -> Error {
        let token = self.current();
        spanned_error(self.source_id, token.start, token.end, code, message)
    }
}

fn is_keyword(word: &str) -> bool {
    matches!(
        word,
        "nexa"
            | "module"
            | "import"
            | "export"
            | "as"
            | "type"
            | "alias"
            | "interface"
            | "impl"
            | "fn"
            | "action"
            | "effects"
            | "let"
            | "var"
            | "state"
            | "return"
            | "if"
            | "else"
            | "match"
            | "for"
            | "in"
            | "while"
            | "break"
            | "continue"
            | "true"
            | "false"
            | "none"
            | "unit"
            | "and"
            | "or"
            | "not"
            | "is"
            | "main"
            | "effect"
            | "requires"
            | "policy"
            | "allow"
            | "deny"
            | "ask"
            | "abstain"
            | "when"
            | "otherwise"
            | "identity"
            | "agent"
            | "group"
            | "grant"
            | "revoke"
            | "pulse"
            | "run"
            | "blueprint"
            | "input"
            | "output"
            | "node"
            | "connect"
            | "after"
            | "set"
            | "memory"
            | "on"
            | "emit"
            | "task"
            | "spawn"
            | "await"
            | "scope"
            | "defer"
    )
}

fn spanned_error(
    source_id: &str,
    start: usize,
    end: usize,
    code: &str,
    message: impl Into<String>,
) -> Error {
    let mut error = Error::new(code, message);
    error.diagnostic.span = Some(SourceSpan {
        source: source_id.to_owned(),
        start: start as u64,
        end: end as u64,
    });
    error
}

#[cfg(test)]
mod tests {
    use super::*;

    fn function(body: &str) -> String {
        format!("nexa 0.3\nmodule examples.syntax\nexport fn sample() -> Int {{\n{body}\n}}\n")
    }

    #[test]
    fn operator_precedence_and_newline_continuation() {
        let source = function(
            "var value = (1\n + 2 * -3)\n\
             value = value +\n 4\n\
             return value",
        );
        let parsed = parse("precedence.nexa", &source).unwrap();
        let [
            Stmt::Bind {
                mutable: true,
                value: Expr::Binary(Binary::Add, left, right),
                ..
            },
            Stmt::Assign { .. },
            Stmt::Return(Expr::Name(name)),
        ] = parsed.functions[0].body.as_slice()
        else {
            panic!("unexpected statements: {:?}", parsed.functions[0].body);
        };
        assert!(matches!(left.as_ref(), Expr::Int(1)));
        assert!(matches!(
            right.as_ref(),
            Expr::Binary(Binary::Multiply, _, operand)
                if matches!(operand.as_ref(), Expr::Unary(Unary::Negate, _))
        ));
        assert_eq!(name, "value");

        for body in [
            "return value\n.member",
            "return value\n+ 1",
            "return 1 < 2 < 3",
        ] {
            assert!(parse("precedence.nexa", &function(body)).is_err());
        }
    }

    #[test]
    fn comments_keep_boundaries_and_blocks_disambiguate_empty_records() {
        let source = function(
            "var value = 0 /* a comment\n\
             with a newline */ value = 1\n\
             if ready {} else if done { value = 2 }\n\
             let empty = Empty {}\n\
             return value",
        );
        let parsed = parse("blocks.nexa", &source).unwrap();
        assert!(matches!(
            parsed.functions[0].body.as_slice(),
            [
                Stmt::Bind { .. },
                Stmt::Assign { .. },
                Stmt::If { condition: Expr::Name(name), then_body, else_body },
                Stmt::Bind { value: Expr::Record(record, fields), .. },
                Stmt::Return(_),
            ] if name == "ready" && then_body.is_empty()
                 && matches!(else_body.as_slice(), [Stmt::If { .. }])
                 && record == "Empty" && fields.is_empty()
        ));
    }

    #[test]
    fn literals_preserve_values_and_invalid_bytes_report_source_offsets() {
        let source = r#"nexa 0.3
module examples.literals
fn integer() -> Int { return -9223372036854775808 }
fn string() -> String { return "\u{1f600}\r\n" }
fn binary() -> Bytes { return hex"fF0d0A00" }
"#;
        let parsed = parse("literals.nexa", source).unwrap();
        assert!(matches!(
            parsed.functions[0].body.as_slice(),
            [Stmt::Return(Expr::Int(i64::MIN))]
        ));
        assert!(matches!(
            parsed.functions[1].body.as_slice(),
            [Stmt::Return(Expr::String(value))] if value == "\u{1f600}\r\n"
        ));
        assert!(matches!(
            parsed.functions[2].body.as_slice(),
            [Stmt::Return(Expr::Bytes(value))] if value == &[255, 13, 10, 0]
        ));

        let invalid = source.replace("fF0d0A00", "f");
        let diagnostic = parse("literals.nexa", &invalid).unwrap_err().diagnostic;
        assert_eq!(diagnostic.code, "E_BYTES");
        let span = diagnostic.span.unwrap();
        assert_eq!(span.source, "literals.nexa");
        assert_eq!(&invalid[span.start as usize..span.end as usize], "hex\"f\"");
    }

    #[test]
    fn rejects_deep_grouping_and_flat_expression_chains() {
        let depth = MAX_SYNTAX_NESTING + 1;
        let grouping = format!("return {}0{}", "(".repeat(depth), ")".repeat(depth));
        let arithmetic = format!("return {}0", "0 + ".repeat(MAX_EXPRESSION_DEPTH + 1));
        let members = format!("return value{}", ".field".repeat(MAX_EXPRESSION_DEPTH + 1));
        for body in [grouping, arithmetic, members] {
            let diagnostic = parse("limit.nexa", &function(&body))
                .unwrap_err()
                .diagnostic;
            assert_eq!(diagnostic.code, "E_NESTING_LIMIT");
        }
    }

    #[test]
    fn enforces_byte_and_token_counts() {
        for source in [" ".repeat(SOURCE_LIMIT + 1), ";".repeat(MAX_TOKENS + 1)] {
            let diagnostic = parse("limit.nexa", &source).unwrap_err().diagnostic;
            assert_eq!(diagnostic.code, "E_SOURCE_LIMIT");
        }
    }
}
