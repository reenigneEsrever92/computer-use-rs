//! The input backend contract.

use super::capture::BoxFuture;
use crate::error::Error;

/// A pointer button, named rather than numbered so callers never deal in evdev
/// button codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Button {
    /// The primary button.
    Left,
    /// The middle button.
    Middle,
    /// The secondary button.
    Right,
}

/// The axis a scroll applies to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Axis {
    /// Up and down.
    Vertical,
    /// Left and right.
    Horizontal,
}

/// A way to act on the desktop.
///
/// Coordinates are always absolute and always in logical desktop coordinates,
/// the same space [`super::capture::CaptureInfo`] reports in.
pub trait InputBackend: Send + Sync + 'static {
    /// Short name of the implementation, used in diagnostics.
    fn name(&self) -> &'static str;

    /// Establish the session, prompting for consent if the platform requires it.
    ///
    /// Idempotent. Where the platform allows it, this shares the consent of the
    /// capture session rather than prompting a second time.
    fn connect(&self) -> BoxFuture<'_, Result<(), Error>>;

    /// Move the pointer to an absolute position.
    fn pointer_move(&self, x: f64, y: f64) -> BoxFuture<'_, Result<(), Error>>;

    /// Click `button` `count` times at the pointer's current position.
    fn pointer_button(&self, button: Button, count: u8) -> BoxFuture<'_, Result<(), Error>>;

    /// Scroll by `amount` along `axis`.
    fn pointer_scroll(&self, axis: Axis, amount: f64) -> BoxFuture<'_, Result<(), Error>>;

    /// Type a string.
    ///
    /// Takes an owned string rather than a borrow so the returned future does not
    /// tie the caller's lifetime to the backend call.
    fn type_text(&self, text: String) -> BoxFuture<'_, Result<(), Error>>;

    /// Press a key combination, written as in `ctrl+shift+t`.
    ///
    /// Separate from [`InputBackend::type_text`]: typing a string is
    /// layout- and Unicode-sensitive, pressing a shortcut is not, and one tool
    /// trying to be both would do neither well.
    fn key_press(&self, combination: String) -> BoxFuture<'_, Result<(), Error>>;
}
