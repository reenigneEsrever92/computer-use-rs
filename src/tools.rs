//! The observation tools: what the model can see, and what it needs to know to
//! act on it.
//!
//! This module is translation, not capture. It turns a captured frame into MCP
//! image content, and capture geometry into coordinates an agent can feed back to
//! the input tools. The interesting part is the coordinates: three spaces exist —
//! the logical desktop, which is what input accepts; stream pixels, which is what
//! the compositor delivers; and image pixels, which is what the agent actually
//! looks at — and only the last of those is the one the agent can measure from.
//! Every number reported here exists to convert between them.

use std::time::Duration;

use base64::Engine as _;
use image::ImageEncoder as _;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::backend::capture::{CaptureInfo, Frame};
use crate::backend::input::{Axis, Button};
use crate::cli::{DEFAULT_MAX_DIMENSION, ImageFormat};
use crate::error::Error;

/// The JPEG quality used when the caller does not ask for one.
pub const DEFAULT_JPEG_QUALITY: u8 = 80;

/// The filter used when downscaling.
///
/// A triangle filter is the compromise: good enough that UI text stays readable
/// after a reduction, and fast enough to stay inside the latency target on a
/// full-screen frame. Lanczos would be sharper and several times slower.
const RESIZE_FILTER: image::imageops::FilterType = image::imageops::FilterType::Triangle;

/// The process-level defaults the tools fall back on.
///
/// These are the command-line options, so that `--max-dimension` and `--format`
/// mean something rather than being accepted and ignored.
#[derive(Debug, Clone, Copy)]
pub struct ToolDefaults {
    /// Long-edge cap applied when the caller does not ask for one.
    pub max_dimension: u32,
    /// Image format used when the caller does not ask for one.
    pub format: ImageFormat,
}

impl Default for ToolDefaults {
    fn default() -> Self {
        Self {
            max_dimension: DEFAULT_MAX_DIMENSION,
            format: ImageFormat::Png,
        }
    }
}

/// The format a screenshot is returned in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ScreenshotFormat {
    /// Lossless. The default, because an agent reading text off a UI needs it.
    Png,
    /// Lossy and smaller, for when cost matters more than fidelity.
    Jpeg,
}

impl From<ScreenshotFormat> for ImageFormat {
    fn from(format: ScreenshotFormat) -> Self {
        match format {
            ScreenshotFormat::Png => Self::Png,
            ScreenshotFormat::Jpeg => Self::Jpeg,
        }
    }
}

/// The arguments `screenshot` accepts.
#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct ScreenshotArgs {
    /// Long-edge cap for the returned image, in pixels. The image is never
    /// enlarged, so a smaller desktop comes back at its own size.
    #[serde(default)]
    pub max_dimension: Option<u32>,

    /// Image format: `png` (default, lossless) or `jpeg`.
    #[serde(default)]
    pub format: Option<ScreenshotFormat>,

    /// JPEG quality, 1 to 100. Ignored for PNG.
    #[serde(default)]
    pub quality: Option<u8>,
}

/// What a screenshot reports alongside its image.
///
/// Positions are in logical desktop coordinates unless a field says otherwise,
/// and `image_to_logical_scale` is the one number an agent needs to act on what
/// it sees.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct ScreenshotMetadata {
    /// The captured region's position in the logical desktop layout.
    pub origin: (i32, i32),
    /// The captured region's size in logical desktop coordinates.
    pub logical_size: (u32, u32),
    /// The size of the frame the compositor delivered, in pixels.
    pub stream_size: (u32, u32),
    /// Stream pixels per logical unit, from fractional display scaling.
    pub scale_factor: f64,
    /// The size of the image returned with this metadata, in pixels.
    pub image_size: (u32, u32),
    /// Multiply a position in the returned image by this to get logical desktop
    /// coordinates — which is what the input tools accept.
    pub image_to_logical_scale: f64,
    /// Pointer position in logical desktop coordinates, when known.
    pub pointer: Option<(f64, f64)>,
    /// The image format returned.
    pub format: String,
    /// How long the whole call took, for an agent budgeting its loop.
    pub elapsed_ms: u64,
}

impl ScreenshotMetadata {
    /// Describe a frame that was captured and then reduced to `image_size`.
    pub fn new(
        info: &CaptureInfo,
        frame_size: (u32, u32),
        image_size: (u32, u32),
        format: ImageFormat,
        elapsed: Duration,
    ) -> Self {
        Self {
            origin: info.origin,
            logical_size: info.logical_size,
            stream_size: info.stream_size,
            scale_factor: info.scale_factor,
            image_size,
            image_to_logical_scale: image_to_logical_scale(
                frame_size,
                image_size,
                info.scale_factor,
            ),
            pointer: info.pointer,
            format: format.mime_type().to_string(),
            elapsed_ms: elapsed.as_millis() as u64,
        }
    }
}

/// The arguments `mouse_move` accepts.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct MouseMoveArgs {
    /// Horizontal position in logical desktop coordinates.
    pub x: f64,
    /// Vertical position in logical desktop coordinates.
    pub y: f64,
}

/// A pointer button, as named on the command surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ButtonArg {
    /// The primary button.
    Left,
    /// The middle button.
    Middle,
    /// The secondary button.
    Right,
}

impl From<ButtonArg> for Button {
    fn from(button: ButtonArg) -> Self {
        match button {
            ButtonArg::Left => Self::Left,
            ButtonArg::Middle => Self::Middle,
            ButtonArg::Right => Self::Right,
        }
    }
}

/// The arguments `mouse_click` accepts.
#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct MouseClickArgs {
    /// Which button to click. Defaults to the primary button.
    #[serde(default)]
    pub button: Option<ButtonArg>,
    /// How many times to click. Defaults to once; two is a double click.
    #[serde(default)]
    pub count: Option<u8>,
    /// Horizontal position to move to first, in logical desktop coordinates.
    #[serde(default)]
    pub x: Option<f64>,
    /// Vertical position to move to first, in logical desktop coordinates.
    #[serde(default)]
    pub y: Option<f64>,
}

/// The axis a scroll applies to, as named on the command surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum AxisArg {
    /// Up and down.
    Vertical,
    /// Left and right.
    Horizontal,
}

impl From<AxisArg> for Axis {
    fn from(axis: AxisArg) -> Self {
        match axis {
            AxisArg::Vertical => Self::Vertical,
            AxisArg::Horizontal => Self::Horizontal,
        }
    }
}

/// The arguments `mouse_scroll` accepts.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct MouseScrollArgs {
    /// Which axis to scroll along. Defaults to vertical.
    #[serde(default)]
    pub axis: Option<AxisArg>,
    /// How many wheel steps to scroll. Negative scrolls the other way, and
    /// fractions are rounded to whole steps.
    pub amount: f64,
}

/// The arguments `type_text` accepts.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct TypeTextArgs {
    /// The text to type into whatever has focus.
    pub text: String,
}

/// The arguments `key_press` accepts.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct KeyPressArgs {
    /// The combination to press, such as `ctrl+shift+t`, `enter`, or `super`.
    /// Modifiers are `ctrl`, `shift`, `alt`, and `super`.
    pub combination: String,
}

/// What `get_display_info` reports.
///
/// One source is captured, so this describes the captured region rather than a
/// list of monitors: enumerating outputs is deferred, and reporting a list here
/// would claim more than the backend knows.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct DisplayInfo {
    /// The captured region's position in the logical desktop layout.
    pub origin: (i32, i32),
    /// The captured region's size in logical desktop coordinates.
    pub logical_size: (u32, u32),
    /// The size of the frames the compositor delivers, in pixels.
    pub stream_size: (u32, u32),
    /// Stream pixels per logical unit, from fractional display scaling.
    pub scale_factor: f64,
    /// Pointer position in logical desktop coordinates, when known.
    pub pointer: Option<(f64, f64)>,
    /// Whether input injection can work on this session.
    ///
    /// False on a session whose portal backend implements screen casting but not
    /// remote desktop, which is how an agent discovers it cannot act here.
    pub input_available: bool,
    /// The capture backend in use, for diagnostics.
    pub backend: String,
}

impl DisplayInfo {
    /// Describe a session.
    pub fn new(info: &CaptureInfo, backend: &str) -> Self {
        Self {
            origin: info.origin,
            logical_size: info.logical_size,
            stream_size: info.stream_size,
            scale_factor: info.scale_factor,
            pointer: info.pointer,
            input_available: info.input_available,
            backend: backend.to_string(),
        }
    }
}

/// How much to multiply an image position by to reach logical desktop
/// coordinates.
///
/// The image is a reduction of the stream, and the stream is a magnification of
/// the logical desktop, so the two factors compose: an image pixel is worth
/// `stream / image` stream pixels, and each of those is `1 / scale_factor`
/// logical units.
pub fn image_to_logical_scale(
    frame_size: (u32, u32),
    image_size: (u32, u32),
    scale_factor: f64,
) -> f64 {
    if image_size.0 == 0 || frame_size.0 == 0 || !scale_factor.is_finite() || scale_factor <= 0.0 {
        return 1.0;
    }
    let reduction = frame_size.0 as f64 / image_size.0 as f64;
    reduction / scale_factor
}

/// The size to reduce a frame to so its long edge is at most `cap`.
///
/// Never enlarges: a desktop already inside the cap comes back untouched, because
/// inventing pixels would cost more and show less. Never returns a zero
/// dimension either, since an image with none cannot be encoded.
pub fn fit_within(width: u32, height: u32, cap: u32) -> (u32, u32) {
    let long_edge = width.max(height);
    if cap == 0 || long_edge <= cap {
        return (width, height);
    }

    let ratio = cap as f64 / long_edge as f64;
    (
        ((width as f64 * ratio).round() as u32).max(1),
        ((height as f64 * ratio).round() as u32).max(1),
    )
}

/// Reduce a frame to `size`, or hand it back unchanged if it is already that size.
pub fn resize(frame: &Frame, size: (u32, u32)) -> Result<Frame, Error> {
    if (frame.width, frame.height) == size {
        return Ok(frame.clone());
    }

    let source = image::RgbImage::from_raw(frame.width, frame.height, frame.rgb.clone())
        .ok_or_else(|| not_an_image(frame))?;
    let reduced = image::imageops::resize(&source, size.0, size.1, RESIZE_FILTER);

    Ok(Frame {
        width: size.0,
        height: size.1,
        rgb: reduced.into_raw(),
    })
}

/// Encode a frame as PNG or JPEG.
pub fn encode(frame: &Frame, format: ImageFormat, quality: u8) -> Result<Vec<u8>, Error> {
    let mut bytes = Vec::new();
    let (width, height) = (frame.width, frame.height);
    let pixels = frame.rgb.as_slice();

    match format {
        ImageFormat::Png => {
            // The default compression spends a long time on a full-screen image
            // for little gain, and this runs inside an agent's loop.
            image::codecs::png::PngEncoder::new_with_quality(
                &mut bytes,
                image::codecs::png::CompressionType::Fast,
                image::codecs::png::FilterType::Adaptive,
            )
            .write_image(pixels, width, height, image::ExtendedColorType::Rgb8)
            .map_err(|error| {
                Error::Capture(format!("could not encode the image as PNG: {error}"))
            })?;
        }
        ImageFormat::Jpeg => {
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, quality)
                .write_image(pixels, width, height, image::ExtendedColorType::Rgb8)
                .map_err(|error| {
                    Error::Capture(format!("could not encode the image as JPEG: {error}"))
                })?;
        }
    }

    Ok(bytes)
}

/// Base64, which is how MCP carries image bytes.
pub fn to_base64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// The error for a frame whose pixel buffer does not match its dimensions.
fn not_an_image(frame: &Frame) -> Error {
    Error::Capture(format!(
        "the captured frame is not a well-formed image: {} bytes for {}x{}",
        frame.rgb.len(),
        frame.width,
        frame.height
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(width: u32, height: u32) -> Frame {
        Frame {
            width,
            height,
            rgb: vec![0x40; Frame::expected_len(width, height)],
        }
    }

    #[test]
    fn a_frame_inside_the_cap_is_left_alone() {
        assert_eq!(fit_within(1280, 720, 1568), (1280, 720));
        assert_eq!(fit_within(1568, 980, 1568), (1568, 980));
    }

    #[test]
    fn a_frame_over_the_cap_is_reduced_on_its_long_edge() {
        assert_eq!(fit_within(2560, 1600, 1568), (1568, 980));
        assert_eq!(fit_within(1600, 2560, 1280), (800, 1280));
    }

    #[test]
    fn a_portrait_frame_is_reduced_on_its_height() {
        let (width, height) = fit_within(1000, 4000, 1000);
        assert_eq!(height, 1000);
        assert_eq!(width, 250);
    }

    #[test]
    fn a_zero_cap_is_treated_as_no_cap() {
        assert_eq!(fit_within(2560, 1600, 0), (2560, 1600));
    }

    #[test]
    fn a_tiny_cap_still_produces_a_usable_size() {
        let (width, height) = fit_within(2560, 1600, 1);
        assert!(width >= 1 && height >= 1, "{width}x{height}");
    }

    #[test]
    fn resizing_to_the_same_size_changes_nothing() {
        let original = frame(4, 4);
        let same = resize(&original, (4, 4)).expect("a resize");
        assert_eq!(same, original);
    }

    #[test]
    fn resizing_produces_the_requested_size() {
        let reduced = resize(&frame(64, 48), (32, 24)).expect("a resize");
        assert_eq!((reduced.width, reduced.height), (32, 24));
        assert!(reduced.is_well_formed());
    }

    #[test]
    fn the_identity_case_scales_by_one() {
        assert_eq!(image_to_logical_scale((2560, 1600), (2560, 1600), 1.0), 1.0);
    }

    #[test]
    fn a_scaled_desktop_reduces_image_coordinates() {
        // 2560 stream pixels across a 1536-wide logical desktop: one image pixel
        // is worth 1536/2560 = 0.6 logical units.
        let scale = image_to_logical_scale((2560, 1600), (2560, 1600), 2560.0 / 1536.0);
        assert!((scale - 0.6).abs() < 1e-9, "{scale}");
    }

    #[test]
    fn downscaling_is_accounted_for_as_well_as_scaling() {
        // The image is reduced from 2560 to 1568, so one image pixel covers
        // 2560/1568 stream pixels, each worth 0.6 logical units.
        let scale = image_to_logical_scale((2560, 1600), (1568, 980), 2560.0 / 1536.0);
        let expected = (2560.0 / 1568.0) * 0.6;
        assert!((scale - expected).abs() < 1e-9, "{scale}");

        // And the far corner of the image lands on the far corner of the
        // desktop, which is the property that matters.
        assert!((1568.0 * scale - 1536.0).abs() < 1e-6);
    }

    #[test]
    fn a_nonsensical_scale_factor_falls_back_to_one() {
        assert_eq!(image_to_logical_scale((100, 100), (100, 100), 0.0), 1.0);
        assert_eq!(image_to_logical_scale((0, 0), (100, 100), 2.0), 1.0);
        assert_eq!(image_to_logical_scale((100, 100), (0, 0), 2.0), 1.0);
    }

    #[test]
    fn a_png_round_trips_at_the_right_size() {
        let encoded =
            encode(&frame(40, 30), ImageFormat::Png, DEFAULT_JPEG_QUALITY).expect("a PNG");
        let decoded = image::load_from_memory(&encoded).expect("a decodable PNG");
        assert_eq!((decoded.width(), decoded.height()), (40, 30));
    }

    #[test]
    fn a_jpeg_round_trips_at_the_right_size() {
        let encoded =
            encode(&frame(40, 30), ImageFormat::Jpeg, DEFAULT_JPEG_QUALITY).expect("a JPEG");
        let decoded = image::load_from_memory(&encoded).expect("a decodable JPEG");
        assert_eq!((decoded.width(), decoded.height()), (40, 30));
    }

    #[test]
    fn jpeg_is_smaller_than_png_for_photographic_content() {
        // A gradient compresses well as JPEG and poorly as PNG, which is the
        // tradeoff the two formats exist for.
        let mut gradient = frame(200, 200);
        for (index, byte) in gradient.rgb.iter_mut().enumerate() {
            *byte = (index % 251) as u8;
        }

        let png = encode(&gradient, ImageFormat::Png, DEFAULT_JPEG_QUALITY).expect("a PNG");
        let jpeg = encode(&gradient, ImageFormat::Jpeg, 50).expect("a JPEG");
        assert!(
            jpeg.len() < png.len(),
            "jpeg {} vs png {}",
            jpeg.len(),
            png.len()
        );
    }

    #[test]
    fn base64_output_is_standard_base64() {
        assert_eq!(to_base64(b"desktop-use"), "ZGVza3RvcC11c2U=");
    }

    #[test]
    fn the_metadata_reports_both_sizes_and_the_conversion() {
        let info = CaptureInfo {
            logical_size: (1536, 960),
            stream_size: (2560, 1600),
            origin: (0, 0),
            scale_factor: 2560.0 / 1536.0,
            pointer: Some((100.0, 200.0)),
            input_available: true,
        };

        let metadata = ScreenshotMetadata::new(
            &info,
            (2560, 1600),
            (1568, 980),
            ImageFormat::Png,
            Duration::from_millis(123),
        );

        assert_eq!(metadata.logical_size, (1536, 960));
        assert_eq!(metadata.stream_size, (2560, 1600));
        assert_eq!(metadata.image_size, (1568, 980));
        assert_eq!(metadata.pointer, Some((100.0, 200.0)));
        assert_eq!(metadata.format, "image/png");
        assert_eq!(metadata.elapsed_ms, 123);
        assert!((metadata.image_to_logical_scale - (2560.0 / 1568.0) * 0.6).abs() < 1e-9);
    }
}
