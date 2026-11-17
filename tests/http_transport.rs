#![cfg(feature = "http-client")]

use nexa::http_client::{GetRequest, HttpLimits, canonical_origin_grant};
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    thread::{self, JoinHandle},
    time::Duration,
};

fn request(url: &str) -> GetRequest {
    GetRequest::parse(url).unwrap()
}

fn limits(max_body_bytes: usize) -> HttpLimits {
    HttpLimits {
        timeout: Duration::from_secs(2),
        max_body_bytes,
    }
}

fn start_server(response: Vec<u8>) -> (String, JoinHandle<String>) {
    start_delayed_server(response, Duration::ZERO)
}

fn start_delayed_server(response: Vec<u8>, delay: Duration) -> (String, JoinHandle<String>) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let address = listener.local_addr().unwrap();
    let worker = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let received = read_request(&mut stream);
        if !delay.is_zero() {
            thread::sleep(delay);
        }
        let _ = stream.write_all(&response);
        let _ = stream.flush();
        received
    });
    (format!("http://{address}"), worker)
}

fn start_delayed_body_server(body: Vec<u8>, delay: Duration) -> (String, JoinHandle<String>) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let address = listener.local_addr().unwrap();
    let worker = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let received = read_request(&mut stream);
        let header = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        let _ = stream.write_all(header.as_bytes());
        let _ = stream.flush();
        thread::sleep(delay);
        let _ = stream.write_all(&body);
        let _ = stream.flush();
        received
    });
    (format!("http://{address}"), worker)
}

fn read_request(stream: &mut TcpStream) -> String {
    let mut request = Vec::new();
    let mut chunk = [0_u8; 2048];
    loop {
        match stream.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(count) => {
                request.extend_from_slice(&chunk[..count]);
                if request.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }
        }
    }
    String::from_utf8_lossy(&request).into_owned()
}

fn response(status: &str, body: &[u8]) -> Vec<u8> {
    let mut output = format!(
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )
    .into_bytes();
    output.extend_from_slice(body);
    output
}

#[test]
fn sends_get_with_query_and_decodes_unicode_utf8() {
    let body = "héllo 🐈".as_bytes();
    let (base, worker) = start_server(response("200 OK", body));
    let client = request(&format!("{base}/hello?q=two%20words"));

    assert_eq!(client.execute(limits(body.len())).unwrap(), "héllo 🐈");
    let received = worker.join().unwrap();
    assert!(received.starts_with("GET /hello?q=two%20words HTTP/1.1\r\n"));
}

#[test]
fn rejects_non_success_statuses() {
    let (base, worker) = start_server(response("404 Not Found", b"missing"));
    let error = request(&format!("{base}/missing"))
        .execute(limits(100))
        .unwrap_err();
    worker.join().unwrap();
    assert_eq!(error.diagnostic.code, "E_HTTP_STATUS");
    assert!(error.diagnostic.message.contains("404"));
}

#[test]
fn does_not_follow_redirects_or_contact_the_redirect_target() {
    let second = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    second.set_nonblocking(true).unwrap();
    let second_address = second.local_addr().unwrap();
    let first_response = format!(
        "HTTP/1.1 302 Found\r\nLocation: http://{second_address}/should-not-run\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    )
    .into_bytes();
    let (base, worker) = start_server(first_response);

    let error = request(&format!("{base}/redirect"))
        .execute(limits(100))
        .unwrap_err();
    worker.join().unwrap();
    assert_eq!(error.diagnostic.code, "E_HTTP_STATUS");
    assert!(
        matches!(second.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock)
    );
}

#[test]
fn enforces_body_limit_for_chunked_responses_without_content_length() {
    let chunked = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n5\r\n12345\r\n0\r\n\r\n".to_vec();
    let (base, worker) = start_server(chunked);

    let error = request(&format!("{base}/chunked"))
        .execute(limits(4))
        .unwrap_err();
    worker.join().unwrap();
    assert_eq!(error.diagnostic.code, "E_HTTP_BODY_LIMIT");
}

#[test]
fn enforces_body_limit_for_connection_close_responses_without_length() {
    let mut no_length = b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n".to_vec();
    no_length.extend_from_slice(b"12345");
    let (base, worker) = start_server(no_length);

    let error = request(&format!("{base}/close-delimited"))
        .execute(limits(4))
        .unwrap_err();
    worker.join().unwrap();
    assert_eq!(error.diagnostic.code, "E_HTTP_BODY_LIMIT");
}

#[test]
fn rejects_invalid_utf8_response_body() {
    let (base, worker) = start_server(response("200 OK", &[0xff, 0xfe]));
    let error = request(&format!("{base}/invalid-utf8"))
        .execute(limits(16))
        .unwrap_err();
    worker.join().unwrap();
    assert_eq!(error.diagnostic.code, "E_HTTP_UTF8");
}

#[test]
fn applies_a_custom_timeout() {
    let (base, worker) =
        start_delayed_server(response("200 OK", b"late"), Duration::from_millis(250));
    let error = request(&format!("{base}/slow"))
        .execute(HttpLimits {
            timeout: Duration::from_millis(50),
            max_body_bytes: 16,
        })
        .unwrap_err();
    worker.join().unwrap();
    assert_eq!(error.diagnostic.code, "E_HTTP_TIMEOUT");
}

#[test]
fn applies_timeout_while_reading_the_response_body() {
    let (base, worker) = start_delayed_body_server(b"late".to_vec(), Duration::from_millis(250));
    let error = request(&format!("{base}/slow-body"))
        .execute(HttpLimits {
            timeout: Duration::from_millis(50),
            max_body_bytes: 16,
        })
        .unwrap_err();
    worker.join().unwrap();
    assert_eq!(error.diagnostic.code, "E_HTTP_TIMEOUT");
}

#[test]
fn url_parsing_is_pure_and_origins_are_canonical() {
    let request = GetRequest::parse("HTTPS://ExAmPlE.com:443/a?q=1").unwrap();
    assert_eq!(request.origin(), "https://example.com");
    assert_eq!(
        canonical_origin_grant("HTTPS://ExAmPlE.com:443/").unwrap(),
        "https://example.com"
    );
    assert_eq!(
        canonical_origin_grant("http://example.com:80").unwrap(),
        "http://example.com"
    );
}

#[test]
fn rejects_malformed_unsupported_credentialed_and_fragment_urls() {
    for value in [
        "not a url",
        "ftp://example.com/file",
        "file:///tmp/data",
        "http://user:pass@example.com/",
        "http://@example.com/",
        "http://example.com/path#fragment",
    ] {
        assert_eq!(
            GetRequest::parse(value).unwrap_err().diagnostic.code,
            "E_HTTP_URL",
            "{value}"
        );
    }
}

#[test]
fn rejects_non_origin_grants() {
    for value in [
        "http://example.com/path",
        "http://example.com/path/..",
        "http://example.com/?q=1",
        "http://example.com/#fragment",
        "http://user@example.com/",
        "ftp://example.com/",
        "example.com",
        "https://*.example.com",
    ] {
        assert_eq!(
            canonical_origin_grant(value).unwrap_err().diagnostic.code,
            "E_HTTP_URL",
            "{value}"
        );
    }
}

#[test]
fn rejects_a_zero_timeout_without_opening_a_connection() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap();
    let error = request(&format!("http://{address}/"))
        .execute(HttpLimits {
            timeout: Duration::ZERO,
            max_body_bytes: 16,
        })
        .unwrap_err();
    assert_eq!(error.diagnostic.code, "E_HTTP_TIMEOUT");
    assert!(
        matches!(listener.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock)
    );
}
