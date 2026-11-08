//! Reading a web page for `raw_keep`, as `cox --read-page <url>`, through
//! a proxy of the test's own, which plays the web, since the reader reads
//! nothing of the machine it runs on.

use std::io::{BufRead, BufReader, Write as _};
use std::net::{TcpListener, TcpStream};
use std::process::Command;
use std::thread;

use serde_json::Value;

/// A proxy that answers every page itself, as CONNECT tunnels to it:
/// `/article`, an article, and `/to-here`, a redirect to the machine
/// itself.
fn web() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            thread::spawn(move || serve(stream));
        }
    });
    format!("http://{address}")
}

fn serve(mut stream: TcpStream) {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut request = head(&mut reader);
    if request.starts_with("CONNECT ") {
        stream
            .write_all(b"HTTP/1.1 200 Connection established\r\n\r\n")
            .unwrap();
        request = head(&mut reader);
    }
    let path = request.split_whitespace().nth(1).unwrap_or("/").to_owned();
    let answer = if path.ends_with("/article") {
        let body = format!(
            "<html><head><title>Announcing Rust 1.92.0</title></head><body>\
             <nav><a href=\"/\">Home</a></nav><article><h1>Announcing Rust 1.92.0</h1>{}\
             </article></body></html>",
            "<p>The Rust team is happy to announce a new version of Rust, 1.92.0. \
             Rust is a programming language empowering everyone to build reliable and \
             efficient software.</p>"
                .repeat(3)
        );
        format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\n\
             Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
    } else if path.ends_with("/to-here") {
        "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:9/admin\r\n\
         Content-Length: 0\r\nConnection: close\r\n\r\n"
            .to_owned()
    } else {
        "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_owned()
    };
    let _ = stream.write_all(answer.as_bytes());
}

/// The first line of the request `reader` reads, past its headers.
fn head(reader: &mut impl BufRead) -> String {
    let mut first = String::new();
    reader.read_line(&mut first).unwrap();
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap() == 0 || line == "\r\n" {
            break;
        }
    }
    first
}

/// What `cox --read-page <url>` answers, through the proxy `web`.
fn read(web: &str, url: &str) -> Value {
    let output = Command::new(env!("CARGO_BIN_EXE_cox"))
        .args([cox::raw::page::OPTION, url])
        .env_clear()
        .env("HTTP_PROXY", web)
        .output()
        .unwrap();
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn a_page_is_read_whole_and_nothing_of_the_machine() {
    let web = web();
    let read_page = read(&web, "http://rust.example/article");
    let page = &read_page["page"];
    assert_eq!(page["url"], "http://rust.example/article", "{read_page}");
    assert_eq!(page["title"], "Announcing Rust 1.92.0");
    let text = page["text"].as_str().unwrap();
    assert!(
        text.contains("The Rust team is happy to announce"),
        "{text}"
    );
    assert!(!text.contains("Home"), "{text}");
    // Not even through a redirect.
    let redirected = read(&web, "http://rust.example/to-here");
    let error = redirected["error"].as_str().unwrap();
    assert_eq!(
        error,
        "http://127.0.0.1:9/admin is not on the web, but on this machine or its network, \
         which Cox does not read"
    );
    let missing = read(&web, "http://rust.example/none");
    assert_eq!(
        missing["error"],
        "http://rust.example/none answered 404 Not Found"
    );
}
