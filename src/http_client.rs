//! Opt-in, capability-neutral HTTP GET transport.
//!
//! Parsing and origin grant normalization perform no network access. The
//! caller is responsible for authorizing `origin()` before calling `execute`.
//! The adapter contacts origins directly, does not consult proxy environment
//! variables, and keeps no cookies, credentials, retries, or redirect state.
//! HTTPS uses the transport's normal certificate and hostname verification.

use crate::diagnostic::{Error, Result};
use reqwest::{Url, blocking::Client, redirect::Policy};
use std::{io::Read, time::Duration};

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);
const DEFAULT_MAX_BODY_BYTES: usize = 1024 * 1024;

/// Resource limits for one HTTP request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HttpLimits {
    pub timeout: Duration,
    pub max_body_bytes: usize,
}

impl Default for HttpLimits {
    fn default() -> Self {
        Self {
            timeout: DEFAULT_TIMEOUT,
            max_body_bytes: DEFAULT_MAX_BODY_BYTES,
        }
    }
}

/// A syntactically valid HTTP(S) GET request, without DNS or network activity.
#[derive(Debug, Clone)]
pub struct GetRequest {
    url: Url,
    origin: String,
}

impl GetRequest {
    pub fn parse(url: &str) -> Result<Self> {
        let parsed = Url::parse(url)
            .map_err(|_| Error::new("E_HTTP_URL", "expected an absolute HTTP or HTTPS URL"))?;
        validate_http_url(&parsed, url)?;
        let origin = parsed.origin().ascii_serialization();
        Ok(Self {
            url: parsed,
            origin,
        })
    }

    /// Canonical scheme/host/port tuple for authorization matching.
    pub fn origin(&self) -> &str {
        &self.origin
    }

    /// Perform the GET after the caller has authorized `origin()`.
    pub fn execute(&self, limits: HttpLimits) -> Result<String> {
        if limits.timeout.is_zero() {
            return Err(Error::new("E_HTTP_TIMEOUT", "HTTP timeout must be nonzero"));
        }

        let client = Client::builder()
            .no_proxy()
            .redirect(Policy::none())
            .retry(reqwest::retry::never())
            .timeout(limits.timeout)
            .build()
            .map_err(|_| Error::new("E_HTTP_TRANSPORT", "could not initialize HTTP transport"))?;

        let response = client
            .get(self.url.clone())
            .send()
            .map_err(transport_error)?;
        if !response.status().is_success() {
            return Err(Error::new(
                "E_HTTP_STATUS",
                format!("HTTP server returned status {}", response.status().as_u16()),
            ));
        }

        let body = read_bounded(response, limits.max_body_bytes)?;
        String::from_utf8(body)
            .map_err(|_| Error::new("E_HTTP_UTF8", "HTTP response body is not valid UTF-8"))
    }
}

/// Parse an exact HTTP(S) origin grant and return its canonical spelling.
///
/// A root slash is allowed because URL parsers insert it for an origin URL;
/// any other path, query, fragment, or user information is rejected.
pub fn canonical_origin_grant(value: &str) -> Result<String> {
    let url = Url::parse(value)
        .map_err(|_| Error::new("E_HTTP_URL", "expected an absolute HTTP or HTTPS origin"))?;
    validate_http_url(&url, value)?;
    if url.path() != "/"
        || !has_only_root_raw_path(value)
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(Error::new(
            "E_HTTP_URL",
            "origin grant cannot contain a path, query, or fragment",
        ));
    }
    Ok(url.origin().ascii_serialization())
}

fn validate_http_url(url: &Url, original: &str) -> Result<()> {
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err(Error::new(
            "E_HTTP_URL",
            "only absolute HTTP and HTTPS URLs are supported",
        ));
    }
    if url.host_str().is_some_and(|host| host.contains('*')) {
        return Err(Error::new("E_HTTP_URL", "wildcard hosts are not supported"));
    }
    if has_userinfo(url, original) {
        return Err(Error::new(
            "E_HTTP_URL",
            "credentials are not allowed in HTTP URLs",
        ));
    }
    if url.fragment().is_some() {
        return Err(Error::new(
            "E_HTTP_URL",
            "fragments are not allowed in HTTP URLs",
        ));
    }
    Ok(())
}

fn has_userinfo(url: &Url, original: &str) -> bool {
    if !url.username().is_empty() || url.password().is_some() {
        return true;
    }
    // `http://@host/` has an empty username but still contains userinfo.
    raw_authority(original).is_some_and(|(start, end)| original[start..end].contains('@'))
}

fn has_only_root_raw_path(original: &str) -> bool {
    let Some((_, authority_end)) = raw_authority(original) else {
        return false;
    };
    let rest = &original[authority_end..];
    let path_end = rest.find(['?', '#']).unwrap_or(rest.len());
    matches!(&rest[..path_end], "" | "/")
}

fn raw_authority(original: &str) -> Option<(usize, usize)> {
    let authority_start = original.find("://")? + 3;
    let authority_end = original[authority_start..]
        .find(['/', '\\', '?', '#'])
        .map_or(original.len(), |offset| authority_start + offset);
    Some((authority_start, authority_end))
}

fn read_bounded(mut response: reqwest::blocking::Response, limit: usize) -> Result<Vec<u8>> {
    let mut body = Vec::with_capacity(limit.min(16 * 1024));
    let mut chunk = [0_u8; 8192];
    loop {
        let count = response.read(&mut chunk).map_err(body_read_error)?;
        if count == 0 {
            return Ok(body);
        }
        if count > limit.saturating_sub(body.len()) {
            return Err(Error::new(
                "E_HTTP_BODY_LIMIT",
                "HTTP response body exceeds the configured byte limit",
            ));
        }
        body.extend_from_slice(&chunk[..count]);
    }
}

fn transport_error(error: reqwest::Error) -> Error {
    if error.is_timeout() {
        Error::new(
            "E_HTTP_TIMEOUT",
            "HTTP request exceeded its configured timeout",
        )
    } else {
        Error::new("E_HTTP_TRANSPORT", "HTTP request failed")
    }
}

fn body_read_error(error: std::io::Error) -> Error {
    if error.kind() == std::io::ErrorKind::TimedOut
        || error
            .get_ref()
            .and_then(|source| source.downcast_ref::<reqwest::Error>())
            .is_some_and(reqwest::Error::is_timeout)
    {
        Error::new(
            "E_HTTP_TIMEOUT",
            "HTTP request exceeded its configured timeout",
        )
    } else {
        Error::new("E_HTTP_TRANSPORT", "could not read HTTP response body")
    }
}
