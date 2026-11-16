use crate::diagnostic::{Error, Result, SourceSpan};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum TokenKind {
    Word(String),
    Number(String),
    String(String),
    LeftParen,
    RightParen,
    LeftBrace,
    RightBrace,
    Comma,
    Colon,
    Dot,
    Plus,
    Arrow,
    Semicolon,
    Unsupported(char),
    End,
}

#[derive(Debug, Clone)]
pub(super) struct Token {
    pub kind: TokenKind,
    pub span: SourceSpan,
    pub leading_newline: bool,
}

pub(super) fn lex(source_id: &str, text: &str) -> Result<Vec<Token>> {
    Lexer {
        source_id,
        text,
        position: 0,
        pending_newline: false,
        tokens: Vec::new(),
    }
    .run()
}

struct Lexer<'a> {
    source_id: &'a str,
    text: &'a str,
    position: usize,
    pending_newline: bool,
    tokens: Vec<Token>,
}

impl Lexer<'_> {
    fn run(mut self) -> Result<Vec<Token>> {
        while self.position < self.text.len() {
            let Some(ch) = self.peek() else {
                break;
            };
            if ch.is_whitespace() {
                if matches!(ch, '\n' | '\r') {
                    self.pending_newline = true;
                }
                self.advance();
                continue;
            }

            if self.starts_with("//") {
                self.skip_line_comment();
                continue;
            }
            if self.starts_with("/*") {
                self.skip_block_comment()?;
                continue;
            }

            let start = self.position;
            if ch.is_ascii_alphabetic() || ch == '_' {
                self.advance();
                while matches!(self.peek(), Some(next) if next.is_ascii_alphanumeric() || next == '_')
                {
                    self.advance();
                }
                let word = self.text[start..self.position].to_owned();
                self.push(TokenKind::Word(word), start);
                continue;
            }
            if ch.is_ascii_digit() {
                self.lex_number();
                continue;
            }
            if ch == '"' {
                self.lex_string()?;
                continue;
            }

            let kind = if self.starts_with("->") {
                self.position += 2;
                Some(TokenKind::Arrow)
            } else {
                self.advance();
                match ch {
                    '(' => Some(TokenKind::LeftParen),
                    ')' => Some(TokenKind::RightParen),
                    '{' => Some(TokenKind::LeftBrace),
                    '}' => Some(TokenKind::RightBrace),
                    ',' => Some(TokenKind::Comma),
                    ':' => Some(TokenKind::Colon),
                    '.' => Some(TokenKind::Dot),
                    '+' => Some(TokenKind::Plus),
                    ';' => Some(TokenKind::Semicolon),
                    _ if ch.is_ascii_punctuation() => Some(TokenKind::Unsupported(ch)),
                    _ => None,
                }
            };
            if let Some(kind) = kind {
                self.push(kind, start);
            } else {
                return Err(self.error(
                    "E_LEX",
                    format!("unsupported character {ch:?}"),
                    start,
                    self.position,
                ));
            }
        }

        let end = self.text.len();
        self.tokens.push(Token {
            kind: TokenKind::End,
            span: self.span(end, end),
            leading_newline: self.pending_newline,
        });
        Ok(self.tokens)
    }

    fn lex_number(&mut self) {
        let start = self.position;
        while matches!(self.peek(), Some(ch) if ch.is_ascii_digit()) {
            self.advance();
        }
        // Slice 0 has no numeric expressions. Keeping dotted version numbers
        // together makes the `nexa 0.3` header unambiguous.
        while self.peek() == Some('.')
            && self
                .peek_after_current()
                .is_some_and(|ch| ch.is_ascii_digit())
        {
            self.advance();
            while matches!(self.peek(), Some(ch) if ch.is_ascii_digit()) {
                self.advance();
            }
        }
        let number = self.text[start..self.position].to_owned();
        self.push(TokenKind::Number(number), start);
    }

    fn lex_string(&mut self) -> Result<()> {
        let start = self.position;
        self.advance();
        let triple = self.starts_with("\"\"");
        if triple {
            self.position += 2;
        }

        let mut value = String::new();
        loop {
            if self.position >= self.text.len() {
                return Err(self.error(
                    "E_STRING",
                    "unterminated string literal",
                    start,
                    self.position,
                ));
            }
            if triple && self.starts_with("\"\"\"") {
                self.position += 3;
                break;
            }
            if !triple && self.peek() == Some('"') {
                self.advance();
                break;
            }

            let ch = self.peek().expect("position is before end of source");
            if !triple && matches!(ch, '\n' | '\r') {
                return Err(self.error(
                    "E_STRING",
                    "newline in a quoted string; use a triple-quoted string",
                    start,
                    self.position + ch.len_utf8(),
                ));
            }
            if ch != '\\' {
                value.push(ch);
                self.advance();
                continue;
            }

            let escape_start = self.position;
            self.advance();
            let Some(escape) = self.peek() else {
                return Err(self.error(
                    "E_STRING_ESCAPE",
                    "incomplete string escape",
                    escape_start,
                    self.position,
                ));
            };
            self.advance();
            match escape {
                '"' => value.push('"'),
                '\\' => value.push('\\'),
                'n' => value.push('\n'),
                'r' => value.push('\r'),
                't' => value.push('\t'),
                'u' => value.push(self.lex_unicode_escape(escape_start)?),
                _ => {
                    return Err(self.error(
                        "E_STRING_ESCAPE",
                        format!("unsupported string escape \\{escape}"),
                        escape_start,
                        self.position,
                    ));
                }
            }
        }

        self.push(TokenKind::String(value), start);
        Ok(())
    }

    fn lex_unicode_escape(&mut self, escape_start: usize) -> Result<char> {
        if self.peek() != Some('{') {
            return Err(self.error(
                "E_STRING_ESCAPE",
                "Unicode escapes must use \\u{HEX}",
                escape_start,
                self.position,
            ));
        }
        self.advance();
        let digits_start = self.position;
        while matches!(self.peek(), Some(ch) if ch.is_ascii_hexdigit()) {
            self.advance();
        }
        let digits_end = self.position;
        if digits_start == digits_end || self.peek() != Some('}') {
            return Err(self.error(
                "E_STRING_ESCAPE",
                "invalid Unicode escape",
                escape_start,
                self.position,
            ));
        }
        let digits = &self.text[digits_start..digits_end];
        self.advance();
        let mut value = 0_u32;
        for digit in digits.bytes() {
            let digit = (digit as char)
                .to_digit(16)
                .expect("the lexer collected only hexadecimal digits");
            value = value
                .checked_mul(16)
                .and_then(|value| value.checked_add(digit))
                .ok_or_else(|| {
                    self.error(
                        "E_STRING_ESCAPE",
                        "Unicode escape is outside the scalar range",
                        escape_start,
                        self.position,
                    )
                })?;
        }
        char::from_u32(value).ok_or_else(|| {
            self.error(
                "E_STRING_ESCAPE",
                "Unicode escape is not a scalar value",
                escape_start,
                self.position,
            )
        })
    }

    fn skip_line_comment(&mut self) {
        self.position += 2;
        while let Some(ch) = self.peek() {
            if matches!(ch, '\n' | '\r') {
                break;
            }
            self.advance();
        }
    }

    fn skip_block_comment(&mut self) -> Result<()> {
        let start = self.position;
        self.position += 2;
        while self.position < self.text.len() {
            if self.starts_with("*/") {
                self.position += 2;
                return Ok(());
            }
            if matches!(self.peek(), Some('\n' | '\r')) {
                self.pending_newline = true;
            }
            self.advance();
        }
        Err(self.error(
            "E_COMMENT",
            "unterminated block comment",
            start,
            self.position,
        ))
    }

    fn push(&mut self, kind: TokenKind, start: usize) {
        self.tokens.push(Token {
            kind,
            span: self.span(start, self.position),
            leading_newline: std::mem::take(&mut self.pending_newline),
        });
    }

    fn span(&self, start: usize, end: usize) -> SourceSpan {
        SourceSpan {
            source: self.source_id.to_owned(),
            start: start as u64,
            end: end as u64,
        }
    }

    fn error(&self, code: &str, message: impl Into<String>, start: usize, end: usize) -> Error {
        Error {
            diagnostic: crate::diagnostic::Diagnostic {
                code: code.to_owned(),
                message: message.into(),
                path: None,
                span: Some(self.span(start, end)),
            },
        }
    }

    fn starts_with(&self, text: &str) -> bool {
        self.text[self.position..].starts_with(text)
    }

    fn peek(&self) -> Option<char> {
        self.text[self.position..].chars().next()
    }

    fn peek_after_current(&self) -> Option<char> {
        let ch = self.peek()?;
        self.text[self.position + ch.len_utf8()..].chars().next()
    }

    fn advance(&mut self) -> Option<char> {
        let ch = self.peek()?;
        self.position += ch.len_utf8();
        Some(ch)
    }
}
