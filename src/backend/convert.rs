//! Pixel layout conversion.
//!
//! The compositor picks the pixel layout; the rest of the program should not
//! care. Everything downstream of here works with packed RGB, so the conversion
//! happens once, at the edge, and is the only place that knows about padded
//! strides or channel order.

use crate::backend::capture::Frame;

/// A packed 32-bit layout the compositor may deliver.
///
/// These are the layouts the capture stream asks for. `x` is padding — the
/// fourth byte carries no colour information in the `x` variants.
///
/// # On the naming
///
/// Which byte holds red is not obvious from the name. The SPA header says only
/// that "the components are in general described in big-endian order" and refers
/// to the GStreamer documentation, where `RGBx` and `BGRx` are likewise written
/// as bit order rather than byte order. Reading the names as *memory* order
/// therefore risks a red/blue swap that no amount of self-consistency can
/// detect.
///
/// This was settled empirically rather than from the documentation: a capture
/// was compared pixel-for-pixel against a reference PNG that GNOME wrote itself
/// through the portal's `Screenshot` interface (`examples/reference_screenshot.rs`).
/// The memory-order reading won by a factor of 2.6 in mean absolute difference,
/// which is what these variants encode. If a future compositor disagrees, that
/// comparison is the test to re-run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PixelLayout {
    /// Bytes in memory are blue, green, red, padding.
    Bgrx,
    /// Bytes in memory are red, green, blue, padding.
    Rgbx,
    /// Bytes in memory are blue, green, red, alpha.
    Bgra,
    /// Bytes in memory are red, green, blue, alpha.
    Rgba,
}

impl PixelLayout {
    /// Bytes per pixel in the source layout.
    pub const BYTES_PER_PIXEL: usize = 4;

    /// The layout as it appears in a PipeWire format, if it is one we handle.
    pub fn from_video_format(format: pipewire::spa::param::video::VideoFormat) -> Option<Self> {
        use pipewire::spa::param::video::VideoFormat;

        match format {
            VideoFormat::BGRx => Some(Self::Bgrx),
            VideoFormat::RGBx => Some(Self::Rgbx),
            VideoFormat::BGRA => Some(Self::Bgra),
            VideoFormat::RGBA => Some(Self::Rgba),
            _ => None,
        }
    }

    /// Where red, green, and blue live within a pixel.
    ///
    /// The first character of the name is the first byte in memory.
    fn channel_order(self) -> (usize, usize, usize) {
        match self {
            Self::Bgrx | Self::Bgra => (2, 1, 0),
            Self::Rgbx | Self::Rgba => (0, 1, 2),
        }
    }
}

/// Convert one buffer to packed RGB.
///
/// `stride` is the distance between the start of consecutive rows, which is
/// often wider than the row's visible pixels; `src` must hold at least
/// `stride * (height - 1) + width * 4` bytes. Returns `None` when the buffer is
/// too small to be the frame it claims to be, which is the honest outcome when
/// the strides and sizes disagree.
pub fn to_rgb(
    layout: PixelLayout,
    src: &[u8],
    width: u32,
    height: u32,
    stride: usize,
) -> Option<Vec<u8>> {
    let width = width as usize;
    let height = height as usize;

    if width == 0 || height == 0 {
        return Some(Vec::new());
    }

    let row_bytes = width * PixelLayout::BYTES_PER_PIXEL;
    // A stride narrower than a row cannot describe this frame.
    let stride = stride.max(row_bytes);

    let last_row_start = stride.checked_mul(height - 1)?;
    let required = last_row_start.checked_add(row_bytes)?;
    if src.len() < required {
        return None;
    }

    let (red, green, blue) = layout.channel_order();
    let mut rgb = Vec::with_capacity(Frame::expected_len(width as u32, height as u32));

    for row in 0..height {
        let row_start = row * stride;
        let row = &src[row_start..row_start + row_bytes];
        for pixel in row.chunks_exact(PixelLayout::BYTES_PER_PIXEL) {
            rgb.push(pixel[red]);
            rgb.push(pixel[green]);
            rgb.push(pixel[blue]);
        }
    }

    Some(rgb)
}

#[cfg(test)]
mod tests {
    use pipewire::spa::param::video::VideoFormat;

    use super::*;

    /// Two pixels per row, so channel order and row advance are both observable.
    fn two_by_two(pixel: [u8; 4], padding: usize) -> Vec<u8> {
        let stride = 2 * 4 + padding;
        let mut buffer = vec![0u8; stride * 2];
        for row in 0..2 {
            for column in 0..2 {
                let start = row * stride + column * 4;
                buffer[start..start + 4].copy_from_slice(&pixel);
            }
        }
        buffer
    }

    #[test]
    fn layouts_map_to_the_formats_we_ask_for() {
        assert_eq!(
            PixelLayout::from_video_format(VideoFormat::BGRx),
            Some(PixelLayout::Bgrx)
        );
        assert_eq!(
            PixelLayout::from_video_format(VideoFormat::RGBx),
            Some(PixelLayout::Rgbx)
        );
        assert_eq!(
            PixelLayout::from_video_format(VideoFormat::BGRA),
            Some(PixelLayout::Bgra)
        );
        assert_eq!(
            PixelLayout::from_video_format(VideoFormat::RGBA),
            Some(PixelLayout::Rgba)
        );
    }

    #[test]
    fn formats_we_did_not_ask_for_are_rejected() {
        assert_eq!(PixelLayout::from_video_format(VideoFormat::I420), None);
        assert_eq!(PixelLayout::from_video_format(VideoFormat::RGB), None);
    }

    #[test]
    fn bgrx_is_swapped_into_rgb() {
        // Memory: blue=0x11, green=0x22, red=0x33, padding.
        let buffer = two_by_two([0x11, 0x22, 0x33, 0xff], 0);
        let rgb = to_rgb(PixelLayout::Bgrx, &buffer, 2, 2, 8).expect("a valid frame");

        assert_eq!(rgb.len(), 12);
        // Two pixels of red, green, blue.
        assert_eq!(
            &rgb,
            &[
                0x33, 0x22, 0x11, 0x33, 0x22, 0x11, 0x33, 0x22, 0x11, 0x33, 0x22, 0x11
            ]
        );
    }

    #[test]
    fn rgbx_needs_no_swapping() {
        let buffer = two_by_two([0x11, 0x22, 0x33, 0xff], 0);
        let rgb = to_rgb(PixelLayout::Rgbx, &buffer, 2, 2, 8).expect("a valid frame");
        assert_eq!(&rgb[0..3], &[0x11, 0x22, 0x33]);
    }

    #[test]
    fn alpha_layouts_ignore_the_fourth_byte() {
        let buffer = two_by_two([0x11, 0x22, 0x33, 0x44], 0);
        let rgba = to_rgb(PixelLayout::Rgba, &buffer, 2, 2, 8).expect("a valid frame");
        let bgra = to_rgb(PixelLayout::Bgra, &buffer, 2, 2, 8).expect("a valid frame");

        // The alpha byte must not leak into the output.
        assert!(!rgba.contains(&0x44));
        assert!(!bgra.contains(&0x44));
        assert_eq!(&rgba[0..3], &[0x11, 0x22, 0x33]);
        assert_eq!(&bgra[0..3], &[0x33, 0x22, 0x11]);
    }

    #[test]
    fn a_wider_stride_advances_rows_correctly() {
        // Padding after each row must be skipped, not read as pixels.
        let buffer = two_by_two([0x01, 0x02, 0x03, 0x04], 4);
        let rgb = to_rgb(PixelLayout::Rgba, &buffer, 2, 2, 12).expect("a valid frame");

        assert_eq!(rgb.len(), 12);
        assert_eq!(&rgb, &[1, 2, 3, 1, 2, 3, 1, 2, 3, 1, 2, 3]);
    }

    #[test]
    fn a_stride_narrower_than_the_row_is_treated_as_tightly_packed() {
        let buffer = two_by_two([0x01, 0x02, 0x03, 0x04], 0);
        // Pass a nonsense stride; the row is read tightly packed instead.
        let rgb = to_rgb(PixelLayout::Rgba, &buffer, 2, 2, 1).expect("a valid frame");
        assert_eq!(&rgb, &[1, 2, 3, 1, 2, 3, 1, 2, 3, 1, 2, 3]);
    }

    #[test]
    fn a_truncated_buffer_is_rejected() {
        let buffer = two_by_two([0x01, 0x02, 0x03, 0x04], 0);
        let truncated = &buffer[..buffer.len() - 1];
        assert_eq!(to_rgb(PixelLayout::Rgba, truncated, 2, 2, 8), None);
    }

    #[test]
    fn empty_dimensions_produce_an_empty_frame() {
        assert_eq!(to_rgb(PixelLayout::Rgba, &[], 0, 0, 0), Some(Vec::new()));
    }
}
