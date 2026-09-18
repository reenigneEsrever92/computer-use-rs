//! The `desktop-use` binary.
//!
//! Everything substantial lives in the library; this is the entry point, which
//! parses the flags, checks that there is a session to serve, and then hands the
//! main thread to either the MCP server or the preview window.

use std::process::ExitCode;
use std::sync::Arc;

use clap::Parser;
use desktop_use::cli::Cli;
use desktop_use::error::Error;
use desktop_use::{backend, logging, mcp, session};
use tracing::{info, warn};

fn main() -> ExitCode {
    let cli = Cli::parse();
    logging::init();

    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!("{error}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<(), Error> {
    session::check_current_env()?;

    if session::find_portal().is_none() {
        warn!(
            "xdg-desktop-portal was not found; capture and input will not be available. \
             Install it together with a backend for your compositor"
        );
    }

    let backends = Arc::new(backend::resolve());
    match &backends.capture {
        Some(capture) => info!("capture backend: {}", capture.name()),
        None => info!("no capture backend is available"),
    }

    let defaults = cli.tool_defaults();

    #[cfg(feature = "preview")]
    {
        if cli.preview {
            return with_preview(backends, defaults);
        }
    }

    #[cfg(not(feature = "preview"))]
    {
        if cli.preview {
            return Err(Error::PreviewFeatureNotCompiled);
        }
    }

    // Without a window the main thread drives the runtime itself, which is the
    // arrangement everything was built and tested under.
    let runtime = tokio::runtime::Runtime::new().map_err(Error::Runtime)?;
    runtime.block_on(mcp::serve(backends, defaults))
}

/// Serve MCP on a worker thread while the preview window holds the main thread.
///
/// The window needs the main thread — a GUI event loop cannot be moved — so it is
/// the server that moves aside.
#[cfg(feature = "preview")]
fn with_preview(
    backends: Arc<backend::Backends>,
    defaults: desktop_use::tools::ToolDefaults,
) -> Result<(), Error> {
    let serving = Arc::clone(&backends);

    let server = std::thread::Builder::new()
        .name("mcp-server".to_string())
        .spawn(move || {
            let runtime = tokio::runtime::Runtime::new().map_err(Error::Runtime)?;
            runtime.block_on(mcp::serve(serving, defaults))
        })
        .map_err(Error::Runtime)?;

    let window = desktop_use::preview::run(Arc::clone(&backends));

    // If the window never opened there is nothing to wait for, and blocking on
    // the server would hang the process for as long as the client stayed
    // connected.
    window?;

    info!("the preview window closed; the session is still open and the server keeps running");

    // The window is gone, but the session and the client are not. Keep serving,
    // which is exactly what running without `--preview` would have done.
    server
        .join()
        .map_err(|_| Error::Mcp("the MCP server thread stopped unexpectedly".to_string()))?
}
