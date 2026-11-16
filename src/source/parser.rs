use crate::{
    diagnostic::{Error, Result, SourceSpan},
    value::ValueType,
};

use super::{
    ast::{CallArgument, CallableKind, Declaration, Expr, ExprKind, Module, Parameter},
    lexer::{Token, TokenKind, lex},
};
use std::collections::BTreeMap;

const MAX_EXPRESSION_NESTING: usize = 256;
const MAX_ADDITIVE_TERMS: usize = 256;

pub(super) fn parse(source_id: &str, text: &str) -> Result<Module> {
    Parser {
        source_id,
        tokens: lex(source_id, text)?,
        position: 0,
        expression_depth: 0,
        parenthesis_depth: 0,
    }
    .parse_module()
}

struct Parser<'a> {
    source_id: &'a str,
    tokens: Vec<Token>,
    position: usize,
    expression_depth: usize,
    parenthesis_depth: usize,
}

impl Parser<'_> {
    fn parse_module(mut self) -> Result<Module> {
        self.expect_word("nexa")?;
        let version_token = self.current().span.clone();
        let version = self.take_number()?;
        if version != "0.3" {
            return Err(self.error_at(
                "E_VERSION",
                format!("unsupported language version {version}; expected 0.3"),
                &version_token,
            ));
        }
        self.require_separator("language header")?;
        self.expect_word("module")?;
        let name = self.parse_path()?;

        let mut declarations = Vec::new();
        if !self.is_end() {
            self.require_separator("module header")?;
        }
        while !self.is_end() {
            declarations.push(self.parse_declaration()?);
            self.require_declaration_separator()?;
        }

        Ok(Module {
            source_id: self.source_id.to_owned(),
            name,
            declarations,
        })
    }

    fn parse_declaration(&mut self) -> Result<Declaration> {
        let start = self.current().span.clone();
        let exported = if self.peek_word("export") {
            self.advance();
            true
        } else {
            false
        };

        let kind = if self.take_word("fn") {
            CallableKind::Function
        } else if self.take_word("action") {
            CallableKind::Action
        } else {
            return Err(self.unsupported_here(
                "expected a function or action declaration; this syntax is outside slice0",
            ));
        };

        let name = self.take_identifier("callable name")?;
        let parameters = self.parse_parameters()?;
        self.expect_arrow()?;
        let result = self.parse_type()?;

        let declared_effects = if self.take_word("effects") {
            if kind == CallableKind::Function {
                return Err(self.error_previous(
                    "E_UNSUPPORTED",
                    "functions are pure in slice0 and cannot declare effects",
                ));
            }
            self.parse_effects()?
        } else if kind == CallableKind::Action {
            return Err(self.error_here(
                "E_EFFECT_SET",
                "actions must declare an effect set in slice0",
            ));
        } else {
            BTreeMap::new()
        };

        self.expect_left_brace()?;
        if !self.take_word("return") {
            return Err(self.unsupported_here(
                "slice0 callable bodies contain one explicit return expression",
            ));
        }
        let body = self.parse_expression()?;
        if matches!(self.current().kind, TokenKind::Plus)
            && self.current().leading_newline
            && self.parenthesis_depth == 0
        {
            return Err(self.error_here(
                "E_UNSUPPORTED",
                "a newline before + separates statements in slice0",
            ));
        }
        self.take_semicolon();
        self.expect_right_brace()?;
        let end = self.previous().span.clone();

        Ok(Declaration {
            exported,
            kind,
            name,
            parameters,
            result,
            declared_effects,
            body,
            span: join_spans(&start, &end),
        })
    }

    fn parse_parameters(&mut self) -> Result<Vec<Parameter>> {
        self.expect_left_paren()?;
        let mut parameters = Vec::new();
        if self.take_right_paren() {
            return Ok(parameters);
        }

        loop {
            let start = self.current().span.clone();
            let name = self.take_identifier("parameter name")?;
            self.expect_word("is")?;
            let ty = self.parse_type()?;
            let end = self.previous().span.clone();
            parameters.push(Parameter {
                name,
                ty,
                span: join_spans(&start, &end),
            });
            if self.take_right_paren() {
                break;
            }
            self.expect_comma()?;
            if self.take_right_paren() {
                break;
            }
        }
        Ok(parameters)
    }

    fn parse_type(&mut self) -> Result<ValueType> {
        let token = self.current().clone();
        let Some(word) = self.word_value() else {
            return Err(self.unsupported_here("slice0 supports only String and Unit types"));
        };
        match word {
            "String" => {
                self.advance();
                Ok(ValueType::String)
            }
            "Unit" => {
                self.advance();
                Ok(ValueType::Unit)
            }
            other => Err(self.error_at(
                "E_UNSUPPORTED",
                format!("type {other} is outside slice0; expected String or Unit"),
                &token.span,
            )),
        }
    }

    fn parse_effects(&mut self) -> Result<BTreeMap<String, SourceSpan>> {
        self.expect_left_brace()?;
        let mut effects = BTreeMap::new();
        if self.take_right_brace() {
            return Ok(effects);
        }
        loop {
            let start = self.current().span.clone();
            let effect = self.parse_path()?;
            let end = self.previous().span.clone();
            if effects
                .insert(effect.clone(), join_spans(&start, &end))
                .is_some()
            {
                return Err(
                    self.error_previous("E_DUPLICATE", format!("duplicate effect {effect}"))
                );
            }
            if self.take_right_brace() {
                break;
            }
            self.expect_comma()?;
            if self.take_right_brace() {
                break;
            }
        }
        Ok(effects)
    }

    fn parse_expression(&mut self) -> Result<Expr> {
        if self.expression_depth >= MAX_EXPRESSION_NESTING {
            return Err(self.error_here(
                "E_NESTING_LIMIT",
                format!("source expression nesting exceeds {MAX_EXPRESSION_NESTING}"),
            ));
        }
        self.expression_depth += 1;
        let result = self.parse_expression_inner();
        self.expression_depth -= 1;
        result
    }

    fn parse_expression_inner(&mut self) -> Result<Expr> {
        let mut expression = self.parse_primary()?;
        let mut additive_terms = 1;
        while matches!(self.current().kind, TokenKind::Plus) {
            if self.current().leading_newline && self.parenthesis_depth == 0 {
                return Err(self.error_here(
                    "E_UNSUPPORTED",
                    "a newline before + separates statements in slice0",
                ));
            }
            additive_terms += 1;
            if additive_terms > MAX_ADDITIVE_TERMS {
                return Err(self.error_here(
                    "E_NESTING_LIMIT",
                    format!("an expression supports at most {MAX_ADDITIVE_TERMS} additive terms"),
                ));
            }
            self.advance();
            let right = self.parse_primary()?;
            let span = join_spans(&expression.span, &right.span);
            expression = Expr {
                kind: ExprKind::Concat(Box::new(expression), Box::new(right)),
                span,
            };
        }
        Ok(expression)
    }

    fn parse_primary(&mut self) -> Result<Expr> {
        let token = self.current().clone();
        match token.kind {
            TokenKind::String(value) => {
                self.advance();
                Ok(Expr {
                    kind: ExprKind::String(value),
                    span: token.span,
                })
            }
            TokenKind::Word(word) => {
                self.advance();
                if word == "unit" {
                    return Ok(Expr {
                        kind: ExprKind::Unit,
                        span: token.span,
                    });
                }
                if matches!(word.as_str(), "true" | "false" | "none") {
                    return Err(self.error_at(
                        "E_UNSUPPORTED",
                        format!("literal {word} is outside slice0"),
                        &token.span,
                    ));
                }
                if is_unsupported_expression_keyword(&word) {
                    return Err(self.error_at(
                        "E_UNSUPPORTED",
                        format!("expression form {word} is outside slice0"),
                        &token.span,
                    ));
                }

                let mut name = word;
                while self.take_dot() {
                    name.push('.');
                    name.push_str(&self.take_identifier("path segment")?);
                }
                if matches!(self.current().kind, TokenKind::Dot) && self.current().leading_newline {
                    return Err(self.error_here(
                        "E_UNSUPPORTED",
                        "a newline before a qualified path segment is not allowed in slice0",
                    ));
                }
                if matches!(self.current().kind, TokenKind::LeftParen)
                    && self.current().leading_newline
                {
                    return Err(self.error_here(
                        "E_UNSUPPORTED",
                        "a newline before a call parenthesis is not allowed in slice0",
                    ));
                }
                if self.take_left_paren() {
                    self.parenthesis_depth += 1;
                    let parsed_arguments = self.parse_arguments_after_open();
                    self.parenthesis_depth -= 1;
                    let arguments = parsed_arguments?;
                    let end = self.previous().span.clone();
                    Ok(Expr {
                        kind: ExprKind::Call { name, arguments },
                        span: join_spans(&token.span, &end),
                    })
                } else {
                    let end = self.previous().span.clone();
                    Ok(Expr {
                        kind: ExprKind::Name(name),
                        span: join_spans(&token.span, &end),
                    })
                }
            }
            TokenKind::LeftParen => {
                self.advance();
                self.parenthesis_depth += 1;
                let parsed_expression = self.parse_expression();
                let parsed_right = self.expect_right_paren();
                self.parenthesis_depth -= 1;
                let expression = parsed_expression?;
                parsed_right?;
                Ok(expression)
            }
            TokenKind::Unsupported(symbol) => Err(self.error_at(
                "E_UNSUPPORTED",
                format!("syntax `{symbol}` is outside slice0"),
                &token.span,
            )),
            _ => Err(self.unsupported_here(
                "expected a string, unit, parameter, call, or concatenation expression",
            )),
        }
    }

    fn parse_arguments_after_open(&mut self) -> Result<Vec<CallArgument>> {
        let mut arguments = Vec::new();
        if self.take_right_paren() {
            return Ok(arguments);
        }
        loop {
            let start = self.current().span.clone();
            let name = if matches!(self.current().kind, TokenKind::Word(_))
                && matches!(&self.peek_n(1).kind, TokenKind::Colon)
            {
                let name = self.take_identifier("argument name")?;
                self.expect_colon()?;
                Some(name)
            } else {
                None
            };
            let expression = self.parse_expression()?;
            let span = join_spans(&start, &expression.span);
            arguments.push(CallArgument {
                name,
                expression,
                span,
            });
            if self.take_right_paren() {
                break;
            }
            self.expect_comma()?;
            if self.take_right_paren() {
                break;
            }
        }
        Ok(arguments)
    }

    fn parse_path(&mut self) -> Result<String> {
        let mut path = self.take_identifier("path")?;
        while self.take_dot() {
            path.push('.');
            path.push_str(&self.take_identifier("path segment")?);
        }
        Ok(path)
    }

    fn take_identifier(&mut self, context: &str) -> Result<String> {
        let token = self.current().clone();
        let TokenKind::Word(value) = token.kind else {
            return Err(self.error_at("E_PARSE", format!("expected {context}"), &token.span));
        };
        if is_keyword(&value) {
            return Err(self.error_at(
                "E_IDENTIFIER",
                format!("reserved word {value} cannot be used as {context}"),
                &token.span,
            ));
        }
        self.advance();
        Ok(value)
    }

    fn take_number(&mut self) -> Result<String> {
        let token = self.current().clone();
        let TokenKind::Number(value) = token.kind else {
            return Err(self.error_at("E_PARSE", "expected version number", &token.span));
        };
        self.advance();
        Ok(value)
    }

    fn expect_word(&mut self, expected: &str) -> Result<()> {
        if self.take_word(expected) {
            Ok(())
        } else {
            Err(self.error_here("E_PARSE", format!("expected {expected}")))
        }
    }

    fn require_separator(&mut self, after: &str) -> Result<()> {
        if self.current().leading_newline {
            return Ok(());
        }
        if matches!(self.current().kind, TokenKind::Semicolon) {
            while matches!(self.current().kind, TokenKind::Semicolon) {
                self.advance();
            }
            return Ok(());
        }
        Err(self.error_here(
            "E_SEPARATOR",
            format!("{after} must be followed by a newline or semicolon"),
        ))
    }

    fn require_declaration_separator(&mut self) -> Result<()> {
        if self.is_end() || self.current().leading_newline {
            return Ok(());
        }
        if matches!(self.current().kind, TokenKind::Semicolon) {
            while matches!(self.current().kind, TokenKind::Semicolon) {
                self.advance();
            }
            return Ok(());
        }
        Err(self.error_here(
            "E_SEPARATOR",
            "declarations must be separated by a newline or semicolon",
        ))
    }

    fn take_word(&mut self, expected: &str) -> bool {
        if self.peek_word(expected) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn peek_word(&self, expected: &str) -> bool {
        matches!(&self.current().kind, TokenKind::Word(word) if word == expected)
    }

    fn word_value(&self) -> Option<&str> {
        match &self.current().kind {
            TokenKind::Word(word) => Some(word),
            _ => None,
        }
    }

    fn expect_arrow(&mut self) -> Result<()> {
        if matches!(self.current().kind, TokenKind::Arrow) {
            self.advance();
            Ok(())
        } else {
            Err(self.error_here("E_PARSE", "expected -> before result type"))
        }
    }

    fn expect_left_brace(&mut self) -> Result<()> {
        if matches!(self.current().kind, TokenKind::LeftBrace) {
            self.advance();
            Ok(())
        } else {
            Err(self.error_here("E_PARSE", "expected {"))
        }
    }

    fn expect_right_brace(&mut self) -> Result<()> {
        if self.take_right_brace() {
            Ok(())
        } else if !self.is_end() {
            Err(self
                .unsupported_here("slice0 callable bodies contain exactly one return expression"))
        } else {
            Err(self.error_here("E_PARSE", "expected }"))
        }
    }

    fn take_right_brace(&mut self) -> bool {
        if matches!(self.current().kind, TokenKind::RightBrace) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn expect_left_paren(&mut self) -> Result<()> {
        if self.take_left_paren() {
            Ok(())
        } else {
            Err(self.error_here("E_PARSE", "expected ("))
        }
    }

    fn take_left_paren(&mut self) -> bool {
        if matches!(self.current().kind, TokenKind::LeftParen) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn expect_right_paren(&mut self) -> Result<()> {
        if self.take_right_paren() {
            Ok(())
        } else {
            Err(self.error_here("E_PARSE", "expected )"))
        }
    }

    fn take_right_paren(&mut self) -> bool {
        if matches!(self.current().kind, TokenKind::RightParen) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn expect_comma(&mut self) -> Result<()> {
        if matches!(self.current().kind, TokenKind::Comma) {
            self.advance();
            Ok(())
        } else {
            Err(self.error_here("E_PARSE", "expected , or closing delimiter"))
        }
    }

    fn expect_colon(&mut self) -> Result<()> {
        if matches!(self.current().kind, TokenKind::Colon) {
            self.advance();
            Ok(())
        } else {
            Err(self.error_here("E_PARSE", "expected : after named argument"))
        }
    }

    fn take_dot(&mut self) -> bool {
        if matches!(self.current().kind, TokenKind::Dot) && !self.current().leading_newline {
            self.advance();
            true
        } else {
            false
        }
    }

    fn take_semicolon(&mut self) -> bool {
        if matches!(self.current().kind, TokenKind::Semicolon) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn is_end(&self) -> bool {
        matches!(self.current().kind, TokenKind::End)
    }

    fn current(&self) -> &Token {
        &self.tokens[self.position]
    }

    fn peek_n(&self, offset: usize) -> &Token {
        &self.tokens[(self.position + offset).min(self.tokens.len() - 1)]
    }

    fn previous(&self) -> &Token {
        &self.tokens[self.position.saturating_sub(1)]
    }

    fn advance(&mut self) -> &Token {
        if !self.is_end() {
            self.position += 1;
        }
        self.previous()
    }

    fn unsupported_here(&self, message: impl Into<String>) -> Error {
        self.error_here("E_UNSUPPORTED", message)
    }

    fn error_here(&self, code: &str, message: impl Into<String>) -> Error {
        self.error_at(code, message, &self.current().span)
    }

    fn error_previous(&self, code: &str, message: impl Into<String>) -> Error {
        self.error_at(code, message, &self.previous().span)
    }

    fn error_at(&self, code: &str, message: impl Into<String>, span: &SourceSpan) -> Error {
        Error {
            diagnostic: crate::diagnostic::Diagnostic {
                code: code.to_owned(),
                message: message.into(),
                path: None,
                span: Some(span.clone()),
            },
        }
    }
}

fn is_keyword(value: &str) -> bool {
    matches!(
        value,
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

fn is_unsupported_expression_keyword(value: &str) -> bool {
    matches!(
        value,
        "let"
            | "var"
            | "state"
            | "if"
            | "else"
            | "match"
            | "for"
            | "while"
            | "break"
            | "continue"
            | "and"
            | "or"
            | "not"
            | "main"
            | "pulse"
            | "run"
            | "task"
            | "spawn"
            | "await"
            | "scope"
            | "defer"
    )
}

fn join_spans(first: &SourceSpan, last: &SourceSpan) -> SourceSpan {
    SourceSpan {
        source: first.source.clone(),
        start: first.start,
        end: last.end,
    }
}
