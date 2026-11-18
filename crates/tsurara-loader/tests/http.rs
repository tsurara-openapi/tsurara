#![cfg(feature = "http")]

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;
use tsurara_loader::{DefaultFetcher, Fetcher, HttpOptions, parse_document};
use url::Url;

/// Mock server that responds once. Returns the header lines of the request it received.
fn serve_once(body: &'static str, delay: Duration) -> (Url, mpsc::Receiver<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut lines = Vec::new();
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            let line = line.trim_end().to_owned();
            if line.is_empty() {
                break;
            }
            lines.push(line);
        }
        tx.send(lines).unwrap();
        thread::sleep(delay);
        let mut stream = stream;
        let _ = write!(
            stream,
            "HTTP/1.1 200 OK\r\ncontent-type: application/yaml\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
            body.len(),
            body
        );
    });
    (
        Url::parse(&format!("http://{addr}/openapi.yaml")).unwrap(),
        rx,
    )
}

#[test]
fn sends_headers_to_the_same_origin() {
    let (url, rx) = serve_once(
        "openapi: 3.1.0\npaths:\n  /a:\n    get:\n      responses:\n        200:\n          description: ok\n",
        Duration::ZERO,
    );
    let fetcher = DefaultFetcher::with_http(HttpOptions {
        headers: vec![("Authorization".into(), "Bearer secret".into())],
        origin: Some(url.origin()),
        timeout: Some(Duration::from_secs(5)),
    });
    let body = fetcher.fetch(&url).unwrap();
    let lines = rx.recv().unwrap();
    assert!(
        lines
            .iter()
            .any(|l| l.eq_ignore_ascii_case("authorization: Bearer secret")),
        "{lines:?}"
    );
    let doc = parse_document(&body).unwrap();
    assert_eq!(
        doc["paths"]["/a"]["get"]["responses"]["200"]["description"],
        "ok"
    );
}

#[test]
fn does_not_send_headers_to_other_origins() {
    let (url, rx) = serve_once("{}", Duration::ZERO);
    let fetcher = DefaultFetcher::with_http(HttpOptions {
        headers: vec![("Authorization".into(), "Bearer secret".into())],
        origin: Some(Url::parse("https://api.example.com").unwrap().origin()),
        timeout: Some(Duration::from_secs(5)),
    });
    fetcher.fetch(&url).unwrap();
    let lines = rx.recv().unwrap();
    assert!(
        !lines
            .iter()
            .any(|l| l.to_ascii_lowercase().starts_with("authorization:")),
        "{lines:?}"
    );
}

#[test]
fn times_out() {
    let (url, _rx) = serve_once("{}", Duration::from_secs(3));
    let fetcher = DefaultFetcher::with_http(HttpOptions {
        timeout: Some(Duration::from_millis(200)),
        ..HttpOptions::default()
    });
    assert!(fetcher.fetch(&url).is_err());
}

/// A server that redirects every request to `location` and reports the request's header lines.
fn redirect_once(location: String) -> (Url, mpsc::Receiver<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut lines = Vec::new();
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            let line = line.trim_end().to_owned();
            if line.is_empty() {
                break;
            }
            lines.push(line);
        }
        tx.send(lines).unwrap();
        let mut stream = stream;
        let _ = write!(
            stream,
            "HTTP/1.1 302 Found\r\nlocation: {location}\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"
        );
    });
    (
        Url::parse(&format!("http://{addr}/openapi.yaml")).unwrap(),
        rx,
    )
}

#[test]
fn headers_do_not_follow_redirects_to_other_origins() {
    let (target, target_rx) = serve_once("{}", Duration::ZERO);
    let (url, rx) = redirect_once(target.to_string());
    let fetcher = DefaultFetcher::with_http(HttpOptions {
        headers: vec![("X-Api-Key".into(), "secret".into())],
        origin: Some(url.origin()),
        timeout: Some(Duration::from_secs(5)),
    });
    fetcher.fetch(&url).unwrap();
    let first = rx.recv().unwrap();
    assert!(
        first
            .iter()
            .any(|l| l.eq_ignore_ascii_case("x-api-key: secret")),
        "{first:?}"
    );
    let second = target_rx.recv().unwrap();
    assert!(
        !second
            .iter()
            .any(|l| l.to_ascii_lowercase().starts_with("x-api-key:")),
        "{second:?}"
    );
}

#[test]
fn redirects_to_local_files_are_refused() {
    let (url, _rx) = redirect_once("file:///etc/hosts".into());
    let fetcher = DefaultFetcher::with_http(HttpOptions {
        timeout: Some(Duration::from_secs(5)),
        ..HttpOptions::default()
    });
    assert!(fetcher.fetch(&url).is_err());
}
