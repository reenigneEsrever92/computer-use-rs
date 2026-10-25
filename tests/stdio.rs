//! End-to-end tests for the stdio transport.
//!
//! These spawn the real binary, so they cover the transport, the logging setup,
//! and the exit path together — the parts a unit test of the handler would miss.
//! The session preflight is satisfied explicitly rather than inherited, so the
//! tests do not depend on the environment they are run in.

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, Command, ExitStatus, Stdio};

/// A command for the binary with a complete session environment.
fn session_command() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_computer-use"));
    command
        .env("DBUS_SESSION_BUS_ADDRESS", "unix:path=/run/user/0/bus")
        .env("XDG_RUNTIME_DIR", "/run/user/0")
        .env("WAYLAND_DISPLAY", "wayland-test");
    command
}

/// A command whose stdio is wired up for a conversation.
fn piped_command() -> Command {
    let mut command = session_command();
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
}

/// The `initialize` request, using the protocol version this build speaks.
fn initialize_request() -> String {
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2025-11-25",
            "capabilities": {},
            "clientInfo": { "name": "computer-use-test", "version": "0" }
        }
    })
    .to_string()
}

/// The notification the protocol requires after `initialize` is answered.
fn initialized_notification() -> String {
    serde_json::json!({
        "jsonrpc": "2.0",
        "method": "notifications/initialized"
    })
    .to_string()
}

/// A `tools/list` request.
fn tools_list_request() -> String {
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/list",
        "params": {}
    })
    .to_string()
}

/// Whether a stdout line is a response, as opposed to a notification.
fn is_response(line: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(line)
        .is_ok_and(|value| value.get("id").is_some_and(|id| !id.is_null()))
}

/// Write the handshake, read both replies, then close stdin and wait.
fn handshake(child: &mut Child) -> (Vec<String>, ExitStatus) {
    let mut stdin = child.stdin.take().expect("stdin was piped");
    let stdout = child.stdout.take().expect("stdout was piped");
    let mut reader = BufReader::new(stdout);

    for request in [
        initialize_request(),
        initialized_notification(),
        tools_list_request(),
    ] {
        writeln!(stdin, "{request}").expect("failed to write a request");
        stdin.flush().expect("failed to flush");
    }

    // Read until both replies have arrived: the answer to `initialize` and the
    // answer to `tools/list`.
    let mut lines: Vec<String> = Vec::new();
    let mut responses = 0usize;
    let mut buffer = String::new();
    while responses < 2 {
        buffer.clear();
        match reader.read_line(&mut buffer) {
            Ok(0) => break,
            Ok(_) => {
                let line = buffer.trim_end().to_string();
                if is_response(&line) {
                    responses += 1;
                }
                lines.push(line);
            }
            Err(error) => panic!("failed to read a reply: {error}"),
        }
    }

    // Closing stdin is how a client goes away; the server should exit cleanly.
    drop(stdin);

    let status = child.wait().expect("the server did not exit");
    (lines, status)
}

/// Parse the replies, keyed by request id.
fn replies_by_id(lines: &[String]) -> Vec<serde_json::Value> {
    lines
        .iter()
        .map(|line| serde_json::from_str(line).expect("a stdout line that is not JSON"))
        .filter(serde_json::Value::is_object)
        .collect()
}

#[test]
fn stdout_carries_only_mcp_traffic() {
    let mut child = piped_command()
        .spawn()
        .expect("failed to spawn computer-use");

    let (lines, status) = handshake(&mut child);
    assert!(status.success(), "the server exited with {status}");
    assert!(!lines.is_empty(), "the server produced no output");

    for line in &lines {
        let value: serde_json::Value = serde_json::from_str(line)
            .unwrap_or_else(|error| panic!("stdout line is not JSON ({error}): {line}"));
        assert_eq!(
            value["jsonrpc"], "2.0",
            "stdout line is not JSON-RPC: {line}"
        );
    }
}

#[test]
fn logs_never_reach_stdout() {
    let mut child = piped_command()
        .spawn()
        .expect("failed to spawn computer-use");

    let (lines, _status) = handshake(&mut child);

    for line in &lines {
        assert!(
            !line.contains("INFO") && !line.contains("WARN") && !line.contains("ERROR"),
            "a log line reached stdout: {line}"
        );
    }
}

#[test]
fn initialize_succeeds() {
    let mut child = piped_command()
        .spawn()
        .expect("failed to spawn computer-use");

    let (lines, _status) = handshake(&mut child);
    let replies = replies_by_id(&lines);

    let initialize = replies
        .iter()
        .find(|reply| reply["id"] == 1)
        .expect("no reply to initialize");
    assert!(
        initialize.get("error").is_none(),
        "initialize failed: {initialize}"
    );
    assert_eq!(
        initialize["result"]["serverInfo"]["name"], "computer-use",
        "unexpected server name: {initialize}"
    );
}

#[test]
fn the_tool_surface_is_listed() {
    let mut child = piped_command()
        .spawn()
        .expect("failed to spawn computer-use");

    let (lines, _status) = handshake(&mut child);
    let replies = replies_by_id(&lines);

    let tools = replies
        .iter()
        .find(|reply| reply["id"] == 2)
        .expect("no reply to tools/list");
    let tools = tools["result"]["tools"]
        .as_array()
        .expect("tools/list did not return an array");

    let names: Vec<&str> = tools
        .iter()
        .filter_map(|tool| tool["name"].as_str())
        .collect();

    for name in [
        "screenshot",
        "zoom",
        "get_display_info",
        "cursor_position",
        "mouse_move",
        "left_click",
        "right_click",
        "middle_click",
        "double_click",
        "triple_click",
        "left_click_drag",
        "left_mouse_down",
        "left_mouse_up",
        "scroll",
        "type",
        "key",
        "hold_key",
        "wait",
        "wait_for_change",
    ] {
        assert!(names.contains(&name), "`{name}` is missing: {names:?}");
    }

    for removed in ["mouse_click", "mouse_scroll", "type_text", "key_press"] {
        assert!(
            !names.contains(&removed),
            "`{removed}` should have been replaced: {names:?}"
        );
    }
}

#[test]
fn closing_stdin_shuts_the_server_down_cleanly() {
    let mut child = piped_command()
        .spawn()
        .expect("failed to spawn computer-use");

    let (_lines, status) = handshake(&mut child);

    assert!(
        status.success(),
        "a client disconnecting should be a clean exit, not {status}"
    );
}

#[test]
fn a_missing_session_variable_is_reported_by_name() {
    let output = session_command()
        .env_remove("WAYLAND_DISPLAY")
        .stdin(Stdio::null())
        .output()
        .expect("failed to run computer-use");

    assert!(
        !output.status.success(),
        "the server should refuse to start without a session"
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("WAYLAND_DISPLAY"),
        "the error did not name the missing variable: {stderr}"
    );
}

/// With the feature on, `--preview` opens a window, which an integration test
/// cannot assert without a display; the feature-off path is the one worth
/// pinning down, because it is what a headless build relies on.
#[cfg(not(feature = "preview"))]
#[test]
fn preview_reports_that_the_feature_was_not_compiled_in() {
    let output = session_command()
        .arg("--preview")
        .stdin(Stdio::null())
        .output()
        .expect("failed to run computer-use");

    assert!(
        !output.status.success(),
        "--preview should be refused in a build without the feature"
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("preview"),
        "the error did not mention the preview feature: {stderr}"
    );
}
