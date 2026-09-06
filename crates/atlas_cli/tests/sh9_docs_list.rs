#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

//! SH9 binary-level scenario (design D8): the real `atlas` binary, run
//! twice — once via the short form (`atlas docs list`) and once via the
//! component-prefixed alias (`atlas acta docs list`) — sends byte-identical
//! requests to a request-recording stub HTTP listener, and produces
//! byte-identical stdout. `atlas --help`'s rendered output is asserted
//! against the same real binary, proving `docs` is grouped under Acta at
//! the binary level (PR2's `render_long_help()` test proves the same claim
//! only at the unit level).
//!
//! No database, no `atlas_server`, and no container are needed: the stub
//! listener below is the only server either invocation ever talks to.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::process::Command;
use std::sync::mpsc;

use atlas_client::Component;

const CANNED_DOCUMENTS_PAGE: &str = r#"{"items":[],"next_cursor":null,"has_more":false}"#;

/// Accepts exactly two HTTP/1.1 requests on one listener, in order,
/// recording each request's full request line and headers as sent over the
/// wire and replying to each with [`CANNED_DOCUMENTS_PAGE`]. Reusing one
/// listener (and therefore one `Host` header) for both invocations means
/// the only thing that can make the two recorded requests differ is
/// `strip_component_prefix`'s own behavior, not an incidental difference in
/// where the stub happened to bind.
/// A CLI that exits without sending a request must fail the test, not park it.
const RECORD_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

fn spawn_two_request_stub() -> (String, mpsc::Receiver<String>, std::thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("stub binds a local port");
    let address = listener.local_addr().expect("stub has a local address");
    let (tx, rx) = mpsc::channel();

    let stub = std::thread::spawn(move || {
        for _ in 0..2 {
            let (mut stream, _) = listener.accept().expect("stub accepts a connection");

            let raw = read_request_head(&mut stream);
            tx.send(raw).expect("stub forwards the recorded request");

            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{CANNED_DOCUMENTS_PAGE}",
                CANNED_DOCUMENTS_PAGE.len()
            )
            .expect("stub writes the canned response");
        }
    });

    (format!("http://{address}"), rx, stub)
}

/// Reads until the end of the request head (`\r\n\r\n`), so the recorded
/// text never depends on how TCP happened to segment the bytes.
fn read_request_head(stream: &mut std::net::TcpStream) -> String {
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 1024];

    loop {
        let read = stream.read(&mut chunk).expect("stub reads the request");
        if read == 0 {
            break;
        }
        bytes.extend_from_slice(&chunk[..read]);
        if bytes.windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
    }

    String::from_utf8_lossy(&bytes).into_owned()
}

/// The recorded request's first line, e.g. `GET /workspaces/w/... HTTP/1.1`.
fn request_line(raw: &str) -> &str {
    raw.lines()
        .next()
        .expect("recorded request has a request line")
}

/// The request line's target (path plus query string), with no HTTP version.
fn request_target(raw: &str) -> &str {
    request_line(raw)
        .split(' ')
        .nth(1)
        .expect("request line has a target")
}

#[test]
fn short_form_and_component_prefixed_alias_send_identical_requests() {
    let (base_url, requests, stub) = spawn_two_request_stub();
    let binary = env!("CARGO_BIN_EXE_atlas");

    let short_form = Command::new(binary)
        .args([
            "docs",
            "list",
            "--workspace",
            "w",
            "--project",
            "p",
            "--base-url",
            &base_url,
            "--token",
            "t",
        ])
        .output()
        .expect("short-form invocation runs");
    assert!(
        short_form.status.success(),
        "short-form invocation must exit 0, stderr: {}",
        String::from_utf8_lossy(&short_form.stderr)
    );
    let short_form_request = requests
        .recv_timeout(RECORD_TIMEOUT)
        .expect("stub recorded the short-form request");

    let prefixed = Command::new(binary)
        .args([
            "acta",
            "docs",
            "list",
            "--workspace",
            "w",
            "--project",
            "p",
            "--base-url",
            &base_url,
            "--token",
            "t",
        ])
        .output()
        .expect("component-prefixed invocation runs");
    assert!(
        prefixed.status.success(),
        "component-prefixed invocation must exit 0, stderr: {}",
        String::from_utf8_lossy(&prefixed.stderr)
    );
    let prefixed_request = requests
        .recv_timeout(RECORD_TIMEOUT)
        .expect("stub recorded the component-prefixed request");
    stub.join()
        .expect("stub thread served both requests and exited");

    assert_eq!(
        short_form_request, prefixed_request,
        "the component-prefixed alias must send byte-identical request bytes to the short form"
    );

    assert_eq!(
        short_form.stdout, prefixed.stdout,
        "both invocations must produce identical stdout"
    );

    let expected_path_suffix = format!(
        "/{}/workspaces/w/projects/p/documents",
        Component::Acta.as_str()
    );
    let target = request_target(&short_form_request);
    let path = target
        .split('?')
        .next()
        .expect("request target has a path component");
    assert!(
        path.ends_with(&expected_path_suffix),
        "expected the recorded path to end with `{expected_path_suffix}`, got `{path}`"
    );
}

#[test]
fn help_shows_docs_grouped_under_acta_at_the_binary_level() {
    let binary = env!("CARGO_BIN_EXE_atlas");

    let output = Command::new(binary)
        .arg("--help")
        .output()
        .expect("`atlas --help` runs");
    assert!(output.status.success(), "`atlas --help` must exit 0");

    let stdout = String::from_utf8(output.stdout).expect("`--help` output is valid UTF-8");
    let acta_start = stdout
        .find("Acta commands:")
        .expect("Acta heading missing from `atlas --help`");
    let acta_section_end = stdout[acta_start..]
        .find("Custos commands:")
        .map(|offset| acta_start + offset)
        .unwrap_or(stdout.len());
    let acta_section = &stdout[acta_start..acta_section_end];

    assert!(
        acta_section.contains("docs"),
        "`docs` must appear under the Acta commands section of `atlas --help`, got:\n{acta_section}"
    );
}
