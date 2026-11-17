use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::fmt;

/// Byte offsets refer to a particular source buffer, never to character indices.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SourceSpan {
    pub source: String,
    pub start: u64,
    pub end: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Diagnostic {
    pub code: String,
    pub message: String,
    pub path: Option<String>,
    pub span: Option<SourceSpan>,
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)?;
        if let Some(path) = &self.path {
            write!(f, " ({path})")?;
        }
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
#[error("{diagnostic}")]
pub struct Error {
    pub diagnostic: Diagnostic,
}

impl Error {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            diagnostic: Diagnostic {
                code: code.into(),
                message: message.into(),
                path: None,
                span: None,
            },
        }
    }

    pub fn at(mut self, path: impl Into<String>) -> Self {
        self.diagnostic.path = Some(path.into());
        self
    }
}

pub type Result<T> = std::result::Result<T, Error>;

/// The CLI contract from specification section 19.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ExitCode {
    Success = 0,
    RuntimeFailure = 1,
    ConstructionError = 2,
    Denied = 3,
    AwaitingApproval = 4,
    SkippedOrCancelled = 5,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum FaultCode {
    Io,
    HttpRequest,
    HttpStatus,
    HttpTimeout,
    HttpBodyLimit,
    HttpUtf8,
    HttpTransport,
    BudgetExceeded,
    BindingUnavailable,
    KernelMismatch,
    IntegrityMismatch,
    ArithmeticOverflow,
}
