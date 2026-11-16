//! Parser and source map for the Slice 0 blueprint source subset.

use crate::{
    contracts::{
        BlueprintDraft, Connection, DraftNode, DraftSchema, EmptyList, Endpoint, Language,
        OrderEdge, Port, Profile, WireDocument,
    },
    diagnostic::{Error, Result, SourceSpan},
    value::{Identifier, ValueType},
};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone)]
pub struct ParsedBlueprint {
    pub draft: BlueprintDraft,
    pub source_map: BlueprintSourceMap,
}

/// Source references stay outside the semantic draft, so comments and layout do
/// not affect ProgramSpec identity.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BlueprintSourceMap {
    pub source: String,
    pub blueprint: Option<SourceSpan>,
    pub inputs: BTreeMap<Identifier, SourceSpan>,
    pub output: Option<SourceSpan>,
    pub nodes: BTreeMap<Identifier, SourceSpan>,
    pub connections: BTreeMap<Connection, SourceSpan>,
    pub order_edges: BTreeMap<OrderEdge, SourceSpan>,
}

pub fn parse_source(source_id: &str, text: &str) -> Result<BlueprintDraft> {
    Ok(parse_source_with_spans(source_id, text)?.draft)
}

pub fn parse_source_with_spans(source_id: &str, text: &str) -> Result<ParsedBlueprint> {
    let tokens = lex(source_id, text)?;
    Parser::new(source_id, text.len(), tokens).file()
}

pub fn decode_request(bytes: &[u8]) -> Result<BlueprintDraft> {
    let value = crate::canonical::parse(bytes)?;
    let mut draft: BlueprintDraft = serde_json::from_value(value)
        .map_err(|error| crate::diagnostic::Error::new("E_CONTRACT", error.to_string()))?;
    validate_node_result_endpoints(
        &draft.connections,
        &draft.nodes.iter().map(|node| node.id.clone()).collect(),
    )?;
    draft.normalize()?;
    Ok(draft)
}

pub(crate) fn validate_node_result_endpoints(
    connections: &[Connection],
    node_ids: &BTreeSet<Identifier>,
) -> Result<()> {
    for connection in connections {
        if let Endpoint::Node { node, port } = &connection.from
            && node_ids.contains(node)
            && port.as_str() != "result"
        {
            return Err(Error::new(
                "E_PORT",
                format!("unknown result port {node}.{port}"),
            ));
        }
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Kind {
    Word(String),
    Number(String),
    String(String),
    Symbol(&'static str),
    Separator,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Token {
    kind: Kind,
    start: usize,
    end: usize,
}

impl Token {
    fn span(&self, source: &str) -> SourceSpan {
        SourceSpan {
            source: source.to_owned(),
            start: self.start as u64,
            end: self.end as u64,
        }
    }
}

fn lex(source: &str, text: &str) -> Result<Vec<Token>> {
    let bytes = text.as_bytes();
    let mut tokens = Vec::new();
    let mut pos = 0;
    let mut paren_depth = 0usize;
    let mut bracket_depth = 0usize;

    while pos < bytes.len() {
        let start = pos;
        match bytes[pos] {
            b' ' | b'\t' | 0x0b | 0x0c => pos += 1,
            b'\r' | b'\n' => {
                if bytes[pos] == b'\r' && bytes.get(pos + 1) == Some(&b'\n') {
                    pos += 2;
                } else {
                    pos += 1;
                }
                if paren_depth == 0 && bracket_depth == 0 {
                    push_separator(&mut tokens, start, pos);
                }
            }
            b';' => {
                pos += 1;
                push_separator(&mut tokens, start, pos);
            }
            b'/' if bytes.get(pos + 1) == Some(&b'/') => {
                pos += 2;
                while pos < bytes.len() && !matches!(bytes[pos], b'\r' | b'\n') {
                    pos += 1;
                }
            }
            b'/' if bytes.get(pos + 1) == Some(&b'*') => {
                pos += 2;
                let mut had_newline = None;
                let mut closed = false;
                while pos < bytes.len() {
                    if bytes[pos] == b'*' && bytes.get(pos + 1) == Some(&b'/') {
                        pos += 2;
                        closed = true;
                        break;
                    }
                    if matches!(bytes[pos], b'\r' | b'\n') {
                        let newline_start = pos;
                        if bytes[pos] == b'\r' && bytes.get(pos + 1) == Some(&b'\n') {
                            pos += 2;
                        } else {
                            pos += 1;
                        }
                        had_newline.get_or_insert((newline_start, pos));
                    } else {
                        pos += 1;
                    }
                }
                if !closed {
                    return Err(error(
                        "E_COMMENT",
                        "unterminated block comment",
                        source,
                        start,
                        bytes.len(),
                    ));
                }
                if paren_depth == 0
                    && bracket_depth == 0
                    && let Some((newline_start, newline_end)) = had_newline
                {
                    push_separator(&mut tokens, newline_start, newline_end);
                }
            }
            b'"' => {
                let (value, end) = string_literal(source, text, pos)?;
                tokens.push(Token {
                    kind: Kind::String(value),
                    start,
                    end,
                });
                pos = end;
            }
            c if c.is_ascii_alphabetic() || c == b'_' => {
                pos += 1;
                while pos < bytes.len()
                    && (bytes[pos].is_ascii_alphanumeric() || bytes[pos] == b'_')
                {
                    pos += 1;
                }
                tokens.push(Token {
                    kind: Kind::Word(text[start..pos].to_owned()),
                    start,
                    end: pos,
                });
            }
            c if c.is_ascii_digit() => {
                pos += 1;
                while pos < bytes.len() && bytes[pos].is_ascii_digit() {
                    pos += 1;
                }
                tokens.push(Token {
                    kind: Kind::Number(text[start..pos].to_owned()),
                    start,
                    end: pos,
                });
            }
            b'-' if bytes.get(pos + 1) == Some(&b'>') => {
                pos += 2;
                tokens.push(Token {
                    kind: Kind::Symbol("->"),
                    start,
                    end: pos,
                });
            }
            b'.' => push_symbol(&mut tokens, &mut pos, "."),
            b',' => push_symbol(&mut tokens, &mut pos, ","),
            b':' => push_symbol(&mut tokens, &mut pos, ":"),
            b'=' => push_symbol(&mut tokens, &mut pos, "="),
            b'{' => push_symbol(&mut tokens, &mut pos, "{"),
            b'}' => push_symbol(&mut tokens, &mut pos, "}"),
            b'(' => {
                paren_depth += 1;
                push_symbol(&mut tokens, &mut pos, "(");
            }
            b')' => {
                paren_depth = paren_depth.saturating_sub(1);
                push_symbol(&mut tokens, &mut pos, ")");
            }
            b'[' => {
                bracket_depth += 1;
                push_symbol(&mut tokens, &mut pos, "[");
            }
            b']' => {
                bracket_depth = bracket_depth.saturating_sub(1);
                push_symbol(&mut tokens, &mut pos, "]");
            }
            _ => {
                let next = text[pos..].chars().next().map_or(1, char::len_utf8);
                return Err(error(
                    "E_SYNTAX",
                    "unexpected character in blueprint source",
                    source,
                    start,
                    start + next,
                ));
            }
        }
    }

    Ok(tokens)
}

fn push_separator(tokens: &mut Vec<Token>, start: usize, end: usize) {
    if !matches!(
        tokens.last().map(|token| &token.kind),
        Some(Kind::Separator)
    ) {
        tokens.push(Token {
            kind: Kind::Separator,
            start,
            end,
        });
    }
}

fn push_symbol(tokens: &mut Vec<Token>, pos: &mut usize, symbol: &'static str) {
    let start = *pos;
    *pos += symbol.len();
    tokens.push(Token {
        kind: Kind::Symbol(symbol),
        start,
        end: *pos,
    });
}

fn string_literal(source: &str, text: &str, start: usize) -> Result<(String, usize)> {
    let bytes = text.as_bytes();
    let mut pos = start + 1;
    let mut value = String::new();

    while pos < bytes.len() {
        match bytes[pos] {
            b'"' => return Ok((value, pos + 1)),
            b'\\' => {
                let escape_start = pos;
                pos += 1;
                let Some(escape) = bytes.get(pos).copied() else {
                    return Err(error(
                        "E_ESCAPE",
                        "unterminated string escape",
                        source,
                        escape_start,
                        bytes.len(),
                    ));
                };
                pos += 1;
                match escape {
                    b'"' => value.push('"'),
                    b'\\' => value.push('\\'),
                    b'n' => value.push('\n'),
                    b'r' => value.push('\r'),
                    b't' => value.push('\t'),
                    b'u' if bytes.get(pos) == Some(&b'{') => {
                        pos += 1;
                        let digits_start = pos;
                        while pos < bytes.len() && bytes[pos].is_ascii_hexdigit() {
                            pos += 1;
                        }
                        if digits_start == pos || bytes.get(pos) != Some(&b'}') {
                            return Err(error(
                                "E_ESCAPE",
                                "expected a Unicode scalar as \\u{HEX}",
                                source,
                                escape_start,
                                pos.min(bytes.len()),
                            ));
                        }
                        let scalar = u32::from_str_radix(&text[digits_start..pos], 16).ok();
                        pos += 1;
                        if let Some(character) = scalar.and_then(char::from_u32) {
                            value.push(character);
                        } else {
                            return Err(error(
                                "E_ESCAPE",
                                "Unicode escape is not a scalar value",
                                source,
                                escape_start,
                                pos,
                            ));
                        }
                    }
                    _ => {
                        return Err(error(
                            "E_ESCAPE",
                            "unsupported string escape",
                            source,
                            escape_start,
                            pos,
                        ));
                    }
                }
            }
            b'\r' | b'\n' => {
                return Err(error(
                    "E_STRING",
                    "newline in a quoted string",
                    source,
                    start,
                    pos + 1,
                ));
            }
            _ => {
                let character = text[pos..].chars().next().expect("valid UTF-8 source");
                value.push(character);
                pos += character.len_utf8();
            }
        }
    }

    Err(error(
        "E_STRING",
        "unterminated string literal",
        source,
        start,
        bytes.len(),
    ))
}

fn error(code: &str, message: impl Into<String>, source: &str, start: usize, end: usize) -> Error {
    let mut error = Error::new(code, message);
    error.diagnostic.span = Some(SourceSpan {
        source: source.to_owned(),
        start: start as u64,
        end: end as u64,
    });
    error
}

struct Parser {
    source: String,
    source_len: usize,
    tokens: Vec<Token>,
    pos: usize,
    source_map: BlueprintSourceMap,
}

impl Parser {
    fn new(source: &str, source_len: usize, tokens: Vec<Token>) -> Self {
        Self {
            source: source.to_owned(),
            source_len,
            tokens,
            pos: 0,
            source_map: BlueprintSourceMap {
                source: source.to_owned(),
                ..BlueprintSourceMap::default()
            },
        }
    }

    fn file(mut self) -> Result<ParsedBlueprint> {
        self.expect_word("nexa")?;
        self.expect_number("0")?;
        self.expect_symbol(".")?;
        self.expect_number("3")?;
        self.expect_separator()?;

        self.expect_word("module")?;
        let module = self.path()?;
        self.expect_separator()?;

        let blueprint_start = self.current_start();
        if !self.consume_word("export") {
            return Err(self.failure("E_EXPORT", "a source blueprint must be exported"));
        }
        self.expect_word("blueprint")?;
        let blueprint_name = self.identifier()?;
        self.expect_symbol("{")?;
        self.source_map.blueprint = Some(SourceSpan {
            source: self.source.clone(),
            start: blueprint_start as u64,
            end: self.previous_end() as u64,
        });

        let mut inputs = Vec::new();
        let mut output = None;
        let mut nodes = Vec::new();
        let mut connections = Vec::new();
        let mut order_edges = Vec::new();

        self.separators();
        while !self.peek_symbol("}") {
            if self.is_eof() {
                return Err(self.failure("E_SYNTAX", "unterminated blueprint body"));
            }
            let item_start = self.current_start();
            match self.peek_word_value() {
                Some("input") => {
                    self.bump();
                    let name = self.identifier()?;
                    self.expect_word("is")?;
                    let r#type = self.port_type()?;
                    if self.source_map.inputs.contains_key(&name) {
                        return Err(self.failure("E_DUPLICATE", "duplicate blueprint input"));
                    }
                    self.source_map
                        .inputs
                        .insert(name.clone(), self.span(item_start, self.previous_end()));
                    inputs.push(Port { name, r#type });
                }
                Some("output") => {
                    self.bump();
                    self.expect_word("result")?;
                    self.expect_word("is")?;
                    let r#type = self.port_type()?;
                    if output.is_some() {
                        return Err(self.failure("E_DUPLICATE", "duplicate blueprint output"));
                    }
                    let port = Port {
                        name: Identifier::new("result")?,
                        r#type,
                    };
                    self.source_map.output = Some(self.span(item_start, self.previous_end()));
                    output = Some(port);
                }
                Some("node") => {
                    self.bump();
                    let id = self.identifier()?;
                    self.expect_symbol("=")?;
                    self.expect_word("memory")?;
                    self.expect_symbol("(")?;
                    let selector = self.string_value()?;
                    self.expect_symbol(",")?;
                    self.expect_word("export")?;
                    self.expect_symbol(":")?;
                    let export_text = self.string_value()?;
                    self.expect_symbol(")")?;
                    if nodes.iter().any(|node: &DraftNode| node.id == id) {
                        return Err(self.failure("E_DUPLICATE", "duplicate blueprint node ID"));
                    }
                    let export = Identifier::new(export_text).map_err(|mut error| {
                        error.diagnostic.span = Some(self.span(item_start, self.previous_end()));
                        error
                    })?;
                    self.source_map
                        .nodes
                        .insert(id.clone(), self.span(item_start, self.previous_end()));
                    nodes.push(DraftNode {
                        id,
                        selector,
                        export,
                    });
                }
                Some("connect") => {
                    self.bump();
                    let from = self.endpoint()?;
                    self.expect_symbol("->")?;
                    let to = self.endpoint()?;
                    let connection = Connection { from, to };
                    self.source_map.connections.insert(
                        connection.clone(),
                        self.span(item_start, self.previous_end()),
                    );
                    connections.push(connection);
                }
                Some("after") => {
                    self.bump();
                    let before = self.identifier()?;
                    self.expect_symbol("->")?;
                    let after = self.identifier()?;
                    let edge = OrderEdge { before, after };
                    self.source_map
                        .order_edges
                        .insert(edge.clone(), self.span(item_start, self.previous_end()));
                    order_edges.push(edge);
                }
                Some("set") => {
                    let token = self.peek().cloned();
                    return Err(self.failure_at(
                        "E_UNSUPPORTED",
                        "constant set bindings are unsupported in slice0",
                        token.as_ref(),
                    ));
                }
                Some(keyword) => {
                    let token = self.peek().cloned();
                    return Err(self.failure_at(
                        "E_UNSUPPORTED",
                        format!("unsupported blueprint item: {keyword}"),
                        token.as_ref(),
                    ));
                }
                None => return Err(self.failure("E_SYNTAX", "expected a blueprint item")),
            }
            if !self.peek_symbol("}") {
                self.expect_separator()?;
                self.separators();
            }
        }
        self.expect_symbol("}")?;
        self.separators();
        if !self.is_eof() {
            return Err(self.failure(
                "E_UNSUPPORTED",
                "a blueprint source file may contain only one exported blueprint",
            ));
        }

        let output = output.ok_or_else(|| self.failure("E_OUTPUT", "missing output result"))?;
        let mut draft = BlueprintDraft {
            schema: DraftSchema::V03,
            language: Language::V03,
            profile: Profile::Slice0,
            entry: format!("{module}.{blueprint_name}"),
            inputs,
            output,
            nodes,
            connections,
            order_edges,
            settings: EmptyList::default(),
            requirements: EmptyList::default(),
        };

        validate_node_result_endpoints(
            &draft.connections,
            &draft.nodes.iter().map(|node| node.id.clone()).collect(),
        )
        .map_err(|mut error| {
            if error.diagnostic.span.is_none() {
                error.diagnostic.span = self.source_map.blueprint.clone();
            }
            error
        })?;
        if let Err(mut error) = draft.normalize() {
            if error.diagnostic.span.is_none() {
                error.diagnostic.span = self.source_map.blueprint.clone();
            }
            return Err(error);
        }

        self.source_map.blueprint = Some(SourceSpan {
            source: self.source.clone(),
            start: blueprint_start as u64,
            end: self.previous_end().min(self.source_len) as u64,
        });

        Ok(ParsedBlueprint {
            draft,
            source_map: self.source_map,
        })
    }

    fn endpoint(&mut self) -> Result<Endpoint> {
        let owner = self.word()?;
        self.expect_symbol(".")?;
        let port = self.identifier()?;
        match owner.as_str() {
            "input" => Ok(Endpoint::Input { port }),
            "output" => Ok(Endpoint::Output { port }),
            _ => Ok(Endpoint::Node {
                node: Identifier::new(owner)?,
                port,
            }),
        }
    }

    fn port_type(&mut self) -> Result<ValueType> {
        let name = self.word()?;
        let r#type = match name.as_str() {
            "String" => ValueType::String,
            "Unit" => ValueType::Unit,
            _ => {
                return Err(self.failure(
                    "E_UNSUPPORTED",
                    format!("unsupported blueprint port type: {name}"),
                ));
            }
        };
        if self.peek_symbol("[") {
            return Err(self.failure(
                "E_UNSUPPORTED",
                "generic blueprint port types are unsupported in slice0",
            ));
        }
        Ok(r#type)
    }

    fn path(&mut self) -> Result<String> {
        let mut path = self.word()?;
        while self.consume_symbol(".") {
            path.push('.');
            path.push_str(&self.word()?);
        }
        Ok(path)
    }

    fn identifier(&mut self) -> Result<Identifier> {
        let token = self.peek().cloned();
        let word = self.word()?;
        Identifier::new(word).map_err(|mut error| {
            error.diagnostic.span = token.as_ref().map(|token| token.span(&self.source));
            error
        })
    }

    fn word(&mut self) -> Result<String> {
        match self.peek().cloned() {
            Some(Token {
                kind: Kind::Word(word),
                ..
            }) => {
                self.bump();
                Ok(word)
            }
            _ => Err(self.failure("E_SYNTAX", "expected an identifier")),
        }
    }

    fn string_value(&mut self) -> Result<String> {
        match self.peek().cloned() {
            Some(Token {
                kind: Kind::String(value),
                ..
            }) => {
                self.bump();
                Ok(value)
            }
            _ => Err(self.failure("E_SYNTAX", "expected a quoted string")),
        }
    }

    fn expect_word(&mut self, expected: &str) -> Result<()> {
        if self.consume_word(expected) {
            Ok(())
        } else {
            Err(self.failure("E_SYNTAX", format!("expected `{expected}`")))
        }
    }

    fn expect_number(&mut self, expected: &str) -> Result<()> {
        if matches!(self.peek().map(|token| &token.kind), Some(Kind::Number(n)) if n == expected) {
            self.bump();
            Ok(())
        } else {
            Err(self.failure("E_VERSION", format!("expected version `{expected}`")))
        }
    }

    fn expect_symbol(&mut self, expected: &'static str) -> Result<()> {
        if self.consume_symbol(expected) {
            Ok(())
        } else {
            Err(self.failure("E_SYNTAX", format!("expected `{expected}`")))
        }
    }

    fn expect_separator(&mut self) -> Result<()> {
        if matches!(self.peek().map(|token| &token.kind), Some(Kind::Separator)) {
            self.bump();
            Ok(())
        } else {
            Err(self.failure("E_SYNTAX", "expected a newline or semicolon"))
        }
    }

    fn consume_word(&mut self, expected: &str) -> bool {
        if matches!(self.peek().map(|token| &token.kind), Some(Kind::Word(word)) if word == expected)
        {
            self.bump();
            true
        } else {
            false
        }
    }

    fn consume_symbol(&mut self, expected: &'static str) -> bool {
        if self.peek_symbol(expected) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn separators(&mut self) {
        while matches!(self.peek().map(|token| &token.kind), Some(Kind::Separator)) {
            self.bump();
        }
    }

    fn peek_word_value(&self) -> Option<&str> {
        match self.peek().map(|token| &token.kind) {
            Some(Kind::Word(word)) => Some(word),
            _ => None,
        }
    }

    fn peek_symbol(&self, symbol: &'static str) -> bool {
        matches!(self.peek().map(|token| &token.kind), Some(Kind::Symbol(actual)) if *actual == symbol)
    }

    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    fn bump(&mut self) -> Option<Token> {
        let token = self.tokens.get(self.pos).cloned();
        if token.is_some() {
            self.pos += 1;
        }
        token
    }

    fn is_eof(&self) -> bool {
        self.pos >= self.tokens.len()
    }

    fn current_start(&self) -> usize {
        self.peek().map_or(self.source_len, |token| token.start)
    }

    fn previous_end(&self) -> usize {
        self.pos
            .checked_sub(1)
            .and_then(|pos| self.tokens.get(pos))
            .map_or(0, |token| token.end)
    }

    fn span(&self, start: usize, end: usize) -> SourceSpan {
        SourceSpan {
            source: self.source.clone(),
            start: start as u64,
            end: end as u64,
        }
    }

    fn failure(&self, code: &str, message: impl Into<String>) -> Error {
        self.failure_at(code, message, self.peek())
    }

    fn failure_at(&self, code: &str, message: impl Into<String>, token: Option<&Token>) -> Error {
        let mut error = Error::new(code, message);
        error.diagnostic.span = Some(token.map_or_else(
            || self.span(self.source_len, self.source_len),
            |token| token.span(&self.source),
        ));
        error
    }
}
