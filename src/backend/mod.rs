//! The backend layer: the contract between the MCP tools and the platform.
//!
//! Capture and input each sit behind a trait, so another windowing system or
//! platform can be added without touching the tool implementations. That
//! separation is what keeps the project from being welded to Wayland, and it is
//! cheapest to establish before any tool exists.

pub mod capture;
pub mod convert;
pub mod frame_source;
pub mod input;
pub mod keys;
pub mod portal;

pub use capture::CaptureBackend;
pub use input::InputBackend;

use std::sync::Arc;

/// The backends a session resolved to.
///
/// Both fields are `Arc` because capture and input share **one** portal session:
/// the consent the user gave authorises both, and a second session would mean
/// asking a second time, which the requirements forbid.
///
/// `input` being present means this build has an input implementation, not that
/// the current session can use it. Whether the session actually granted input
/// devices is a per-session fact — reported by `get_display_info`, and enforced
/// by the input methods themselves.
#[derive(Default)]
pub struct Backends {
    /// The observation backend, if one is available.
    pub capture: Option<Arc<dyn CaptureBackend>>,
    /// The control backend, if one is available.
    pub input: Option<Arc<dyn InputBackend>>,
}

impl Backends {
    /// Whether neither backend is available.
    pub fn is_empty(&self) -> bool {
        self.capture.is_none() && self.input.is_none()
    }
}

/// Resolve the backends for this session.
///
/// One portal session is created and handed to both backends. Negotiation is
/// deferred until a tool actually needs the desktop, so resolving never prompts
/// the user.
pub fn resolve() -> Backends {
    let session = Arc::new(portal::PortalSession::new());

    // `.clone()` rather than `Arc::clone(&session)`: the method call resolves the
    // type first, and the unsizing coercion to the trait object then applies.
    let capture: Arc<dyn CaptureBackend> = session.clone();
    let input: Arc<dyn InputBackend> = session;

    Backends {
        capture: Some(capture),
        input: Some(input),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_resolves_without_touching_the_portal() {
        let backends = resolve();

        assert!(!backends.is_empty());
        assert_eq!(
            backends.capture.as_ref().expect("a capture backend").name(),
            "portal+pipewire"
        );
        assert_eq!(
            backends.input.as_ref().expect("an input backend").name(),
            "portal+pipewire"
        );
    }

    #[test]
    fn capture_and_input_share_one_session() {
        let backends = resolve();

        // The data addresses have to match: one session means one consent prompt,
        // and two sessions would mean two.
        let capture = Arc::as_ptr(backends.capture.as_ref().expect("capture")) as *const u8;
        let input = Arc::as_ptr(backends.input.as_ref().expect("input")) as *const u8;

        assert_eq!(
            capture, input,
            "the two backends should be handles onto the same session"
        );
    }
}
