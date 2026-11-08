//! A local stand-in for the GitHub releases API on 127.0.0.1: serves canned
//! bodies by path, 404 for everything else. No real network.
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;

pub const LATEST: &str = "/repos/svenhartmann/ArchGuard/releases/latest";

/// Starts the server; `routes` gets the base URL and returns path → body.
pub fn serve(routes: impl FnOnce(&str) -> HashMap<String, Vec<u8>>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let routes = routes(&base);
    thread::spawn(move || {
        for stream in listener.incoming() {
            answer(stream.unwrap(), &routes);
        }
    });
    base
}

fn answer(mut stream: TcpStream, routes: &HashMap<String, Vec<u8>>) {
    let mut request = Vec::new();
    let mut buf = [0; 1024];
    while !request.windows(4).any(|w| w == b"\r\n\r\n") {
        let n = stream.read(&mut buf).unwrap();
        if n == 0 {
            break;
        }
        request.extend_from_slice(&buf[..n]);
    }
    let line = String::from_utf8_lossy(&request);
    let path = line.split_whitespace().nth(1).unwrap_or_default();
    let (status, body) = match routes.get(path) {
        Some(body) => ("200 OK", body.as_slice()),
        None => ("404 Not Found", b"".as_slice()),
    };
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(body);
}

/// Release metadata for `tag` listing `assets` as `{base}/download/<name>`.
pub fn latest_json(base: &str, tag: &str, assets: &[&str]) -> Vec<u8> {
    let assets: Vec<String> = assets
        .iter()
        .map(|name| {
            format!(
                "{{\"name\": \"{name}\", \"browser_download_url\": \"{base}/download/{name}\"}}"
            )
        })
        .collect();
    format!(
        "{{\"tag_name\": \"{tag}\", \"html_url\": \"https://example.invalid/{tag}\", \"assets\": [{}]}}",
        assets.join(",")
    )
    .into_bytes()
}
