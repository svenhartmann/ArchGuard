//! A server that accepts but never answers must not hang an update check.

use std::net::TcpListener;
use std::time::{Duration, Instant};

use archguard_update::http;

#[test]
fn a_silent_server_times_out_instead_of_hanging() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/stalled", listener.local_addr().unwrap());
    // Keep accepted connections open without writing a byte.
    let _server = std::thread::spawn(move || {
        let mut open = Vec::new();
        for stream in listener.incoming().flatten() {
            open.push(stream);
        }
    });
    let started = Instant::now();
    let result = http::get_with_stall(&url, 1024, Duration::from_millis(300));
    assert!(result.is_err(), "a silent server must give an error");
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "took {:?}",
        started.elapsed()
    );
}
