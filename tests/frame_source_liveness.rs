//! Checks what happens when a capture stream cannot deliver anything.
//!
//! The failure this guards against is quiet: a stream that dies while a good
//! frame is already published will keep serving that frame, and a stale frame is
//! indistinguishable from a current one to a caller.
//!
//! The test connects to the session's PipeWire daemon directly — no portal, no
//! consent — and points the stream at a node that does not exist, so the stream
//! reaches an error state without a compositor having to be restarted. It needs a
//! running PipeWire, so it steps aside when there is none.

use std::os::fd::OwnedFd;
use std::os::unix::net::UnixStream;
use std::time::Duration;

use desktop_use::backend::frame_source::FrameSource;

/// Most significant bit set, and not `PW_ID_ANY`: a node id no real node has.
const NONEXISTENT_NODE: u32 = 0x7fff_fffe;

/// The session's PipeWire socket, if it is there.
fn pipewire_socket() -> Option<std::path::PathBuf> {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")?;
    let socket = std::path::PathBuf::from(runtime).join("pipewire-0");
    socket.exists().then_some(socket)
}

#[tokio::test]
async fn a_stream_that_cannot_deliver_reports_a_failure_instead_of_a_stale_frame() {
    let Some(socket) = pipewire_socket() else {
        eprintln!("skipping: this session has no PipeWire socket");
        return;
    };

    let connection = UnixStream::connect(&socket).expect("connect to the PipeWire daemon");
    let fd = OwnedFd::from(connection);

    let mut source = FrameSource::start(fd, NONEXISTENT_NODE).expect("the stream should start");

    let outcome = source.capture_within(Duration::from_secs(10)).await;

    assert!(
        outcome.is_err(),
        "a stream with no producer reported a frame: {outcome:?}"
    );
    assert!(
        !source.is_alive(),
        "the source still claims to be alive after failing"
    );
    assert!(
        source.state().frame.is_none(),
        "no frame should have been published"
    );
}

#[tokio::test]
async fn a_source_that_has_been_stopped_is_not_alive() {
    let Some(socket) = pipewire_socket() else {
        eprintln!("skipping: this session has no PipeWire socket");
        return;
    };

    let connection = UnixStream::connect(&socket).expect("connect to the PipeWire daemon");
    let fd = OwnedFd::from(connection);

    let mut source = FrameSource::start(fd, NONEXISTENT_NODE).expect("the stream should start");
    source.stop();

    assert!(
        !source.is_alive(),
        "a stopped source should not report itself as alive"
    );
}
