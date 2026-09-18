//! The capture backend contract.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use crate::error::Error;

/// A future returned by a backend method.
///
/// Backends are used through `dyn`, and an `async fn` in a trait is not
/// dyn-compatible, so the methods return this instead of being declared `async`.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// What the desktop looks like, in logical desktop coordinates.
///
/// Logical coordinates are the one well-defined space the MCP tools speak: a
/// position read from here, or from a captured frame's metadata, is accepted
/// unchanged by the input backend. Translating into the captured stream's pixel
/// space, and into whatever space the platform's injection API expects, is the
/// backend's job — never the agent's.
#[derive(Debug, Clone, PartialEq)]
pub struct CaptureInfo {
    /// Size of the captured region in logical desktop coordinates, as the
    /// platform reports it.
    ///
    /// This is the space input coordinates are expressed in.
    pub logical_size: (u32, u32),
    /// Size of the frames actually delivered, in pixels.
    ///
    /// This differs from [`CaptureInfo::logical_size`] whenever the desktop is
    /// scaled: a 1536x960 logical desktop on a 2560x1600 panel delivers
    /// 2560x1600 frames. Convert with [`CaptureInfo::scale_factor`].
    pub stream_size: (u32, u32),
    /// Position of the captured region within the logical desktop layout.
    pub origin: (i32, i32),
    /// Pixels per logical unit: `stream_size / logical_size`.
    pub scale_factor: f64,
    /// Pointer position in logical desktop coordinates, when known.
    pub pointer: Option<(f64, f64)>,
    /// Whether the session has an input backend that can inject input.
    ///
    /// A session can capture without being able to inject — on a portal backend
    /// that implements screen casting but not remote desktop, for instance — and
    /// an agent needs to be able to discover that before it tries to act.
    pub input_available: bool,
}

/// One captured frame, converted to packed RGB.
///
/// The bytes are tightly packed RGB, three bytes per pixel and row-major, so
/// consumers never have to know which pixel layout the platform chose. The
/// conversion happens once, in the backend.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Packed RGB bytes; see [`Frame::BYTES_PER_PIXEL`].
    pub rgb: Vec<u8>,
}

impl Frame {
    /// Bytes per pixel in [`Frame::rgb`].
    pub const BYTES_PER_PIXEL: usize = 3;

    /// The length [`Frame::rgb`] must have for the given dimensions.
    pub fn expected_len(width: u32, height: u32) -> usize {
        width as usize * height as usize * Self::BYTES_PER_PIXEL
    }

    /// Whether the pixel buffer is consistent with the frame's dimensions.
    pub fn is_well_formed(&self) -> bool {
        self.rgb.len() == Self::expected_len(self.width, self.height)
    }
}

/// A way to observe the desktop.
pub trait CaptureBackend: Send + Sync + 'static {
    /// Short name of the implementation, used in diagnostics.
    fn name(&self) -> &'static str;

    /// Establish the session, prompting for consent if the platform requires it.
    ///
    /// Idempotent: calling it when a session already exists must not prompt
    /// again. This is the call that may block on the user.
    fn connect(&self) -> BoxFuture<'_, Result<CaptureInfo, Error>>;

    /// Capture the most recent frame.
    ///
    /// Returned as a shared handle rather than by value because the backend is
    /// already holding one: the preview window polls this, and copying the pixels
    /// on every poll would be a per-poll cost for no benefit.
    fn capture_frame(&self) -> BoxFuture<'_, Result<Arc<Frame>, Error>>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expected_len_is_three_bytes_per_pixel() {
        assert_eq!(Frame::expected_len(0, 0), 0);
        assert_eq!(Frame::expected_len(1, 1), 3);
        assert_eq!(Frame::expected_len(1920, 1080), 1920 * 1080 * 3);
    }

    #[test]
    fn well_formed_requires_exact_length() {
        let mut frame = Frame {
            width: 2,
            height: 2,
            rgb: vec![0; 12],
        };
        assert!(frame.is_well_formed());

        frame.rgb.push(0);
        assert!(!frame.is_well_formed());

        frame.rgb.truncate(11);
        assert!(!frame.is_well_formed());
    }
}

impl CaptureInfo {
    /// Convert a logical desktop position to the pixel position in a frame.
    ///
    /// The frame the compositor delivers is in stream pixels, so this bridges
    /// what the tools speak and what the portal's absolute pointer motion —
    /// which is specified in the stream's pixels — expects.
    pub fn logical_to_stream(&self, logical: (f64, f64)) -> (f64, f64) {
        (logical.0 * self.scale_factor, logical.1 * self.scale_factor)
    }

    /// Convert a position in frame pixels to logical desktop coordinates.
    pub fn stream_to_logical(&self, stream: (f64, f64)) -> (f64, f64) {
        if self.scale_factor == 0.0 {
            return stream;
        }
        (stream.0 / self.scale_factor, stream.1 / self.scale_factor)
    }
}

#[cfg(test)]
mod coordinate_tests {
    use super::*;

    fn scaled(scale: f64) -> CaptureInfo {
        CaptureInfo {
            logical_size: (1536, 960),
            stream_size: (2560, 1600),
            origin: (0, 0),
            scale_factor: scale,
            pointer: None,
            input_available: true,
        }
    }

    #[test]
    fn conversions_round_trip() {
        let info = scaled(2560.0 / 1536.0);
        for point in [(0.0, 0.0), (100.0, 200.0), (1536.0, 960.0)] {
            let there = info.logical_to_stream(point);
            let back = info.stream_to_logical(there);
            assert!((back.0 - point.0).abs() < 1e-9, "{point:?} -> {back:?}");
            assert!((back.1 - point.1).abs() < 1e-9, "{point:?} -> {back:?}");
        }
    }

    #[test]
    fn the_whole_logical_desktop_maps_to_the_whole_frame() {
        let info = scaled(2560.0 / 1536.0);
        let corner = info.logical_to_stream((1536.0, 960.0));
        assert!((corner.0 - 2560.0).abs() < 1e-9, "{corner:?}");
        assert!((corner.1 - 1600.0).abs() < 1e-9, "{corner:?}");
    }

    #[test]
    fn an_unscaled_desktop_is_the_identity() {
        let info = scaled(1.0);
        assert_eq!(info.logical_to_stream((640.0, 480.0)), (640.0, 480.0));
        assert_eq!(info.stream_to_logical((640.0, 480.0)), (640.0, 480.0));
    }
}
