//! Logging setup.
//!
//! Logs go to stderr and only to stderr. stdout is the MCP channel: a stray log
//! line there corrupts the protocol, so the writer is pinned rather than left to
//! `tracing-subscriber`'s default of stdout.

use tracing_subscriber::EnvFilter;

/// The environment variable that overrides the log filter.
pub const LOG_FILTER_ENV: &str = "COMPUTER_USE_LOG";

/// The filter used when [`LOG_FILTER_ENV`] is unset or invalid.
pub const DEFAULT_LOG_FILTER: &str = "info";

/// Install the global subscriber, writing to stderr.
///
/// Safe to call more than once; later calls are ignored, which keeps tests that
/// share a process from panicking.
pub fn init() {
    let filter = EnvFilter::try_from_env(LOG_FILTER_ENV)
        .or_else(|_| EnvFilter::try_new(DEFAULT_LOG_FILTER))
        .unwrap_or_else(|_| EnvFilter::new(DEFAULT_LOG_FILTER));

    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .try_init();
}
