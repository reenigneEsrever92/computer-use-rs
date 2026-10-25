//! `computer-use` — an MCP server that lets an AI agent observe and control a
//! Wayland Linux desktop.
//!
//! The crate is a library so that the binary, the examples, and the tests all
//! share one implementation. The binary is a thin shell around
//! [`mcp::serve`]; the parts worth reusing are the backend traits in
//! [`backend`], which is where a different windowing system or platform would be
//! added.

pub mod backend;
pub mod cli;
pub mod error;
pub mod logging;
pub mod mcp;
#[cfg(feature = "preview")]
pub mod preview;
pub mod session;
pub mod tools;
