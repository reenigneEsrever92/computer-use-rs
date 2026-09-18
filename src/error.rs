//! The error type shared by the CLI, the session preflight, and the backends.

/// Anything that prevents `desktop-use` from serving the desktop.
///
/// Every variant is meant to be actionable on its own. The point of the
/// diagnostics requirement is that a failure names the missing component and
/// what to do about it, rather than surfacing a raw D-Bus error.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A required environment variable of the graphical session is absent.
    #[error(
        "not running inside a graphical session: {variable} is not set. \
         desktop-use must run inside the user's graphical session, not as a system service"
    )]
    MissingSessionEnv { variable: &'static str },

    /// The async runtime could not be started.
    #[error("failed to start the async runtime: {0}")]
    Runtime(#[source] std::io::Error),

    /// The MCP server failed.
    #[error("the MCP server failed: {0}")]
    Mcp(String),

    /// A backend was asked for but none is available.
    #[error("no {backend} backend is available: {detail}")]
    BackendUnavailable {
        backend: &'static str,
        detail: &'static str,
    },

    /// The portal refused, or could not provide, a session.
    #[error("the portal could not provide a {interface} session: {detail}")]
    Portal {
        interface: &'static str,
        detail: String,
    },

    /// The portal frontend, or the backend for this compositor, is not there.
    ///
    /// This is the wlroots case for input: the frontend is present and does
    /// screen casting, but nothing implements remote desktop, so asking for it
    /// fails where asking for capture would not.
    #[error(
        "this session's portal does not provide {interface}: {detail}. Install the backend \
         for your compositor, or use a compositor whose backend implements it"
    )]
    PortalInterfaceUnavailable {
        interface: &'static str,
        detail: String,
    },

    /// The portal refused an event on a live session.
    ///
    /// Distinct from [`Error::Portal`], which is about negotiating the session:
    /// here the session exists and the event itself was rejected — a pointer
    /// position outside the range the portal accepts, for instance.
    #[error("the portal could not inject input: {detail}")]
    Input { detail: String },

    /// The user dismissed the consent prompt.
    #[error(
        "the screen sharing request was dismissed or denied. desktop-use needs the \
         user to approve the session"
    )]
    ConsentDenied,

    /// PipeWire failed.
    #[error("PipeWire failed: {0}")]
    Pipewire(#[from] pipewire::Error),

    /// The capture thread stopped before it reported whether setup succeeded.
    #[error("the PipeWire capture thread stopped during setup")]
    CaptureThreadStopped,

    /// Capture is not working, for a reason only known at runtime.
    #[error("capture failed: {0}")]
    Capture(String),

    /// No frame arrived in time.
    #[error("no frame arrived from the compositor within {seconds} s")]
    FrameTimeout { seconds: u64 },

    /// `--preview` was asked for in a build that has no preview window.
    #[error(
        "--preview needs the `preview` feature, which this build was made without. Rebuild \
         without --no-default-features, or drop --preview"
    )]
    PreviewFeatureNotCompiled,

    /// The preview window could not be opened.
    #[error("could not open the preview window: {0}")]
    PreviewWindow(String),
}

/// The portal interface that provides input injection.
///
/// Named here because the error vocabulary refers to it: whether a session can
/// inject input at all is reported by which interface turned out to be missing.
pub const REMOTE_DESKTOP_INTERFACE: &str = "org.freedesktop.portal.RemoteDesktop";

impl Error {
    /// Whether this is the portal lacking input support, rather than any other
    /// failure.
    ///
    /// The capture-only fallback keys off this, so it must not fire for a denied
    /// consent prompt, or for a missing screen casting interface — there would be
    /// nothing left to fall back to.
    pub fn is_input_unsupported(&self) -> bool {
        matches!(
            self,
            Self::PortalInterfaceUnavailable { interface, .. }
                if *interface == REMOTE_DESKTOP_INTERFACE
        )
    }
}
