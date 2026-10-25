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
use crate::backend::input::Axis;
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

    /// Describe a region of a frame that was cropped and then reduced to fit.
    ///
    /// `origin` and `logical_size` place the region in logical desktop
    /// coordinates, and `stream_size` is how large that region is in the frame's
    /// pixels. `image_to_logical_scale` is computed from `stream_size` and the
    /// returned image, so a position read off a zoomed image maps back onto the
    /// desktop exactly as one read off a full screenshot does.
    #[allow(clippy::too_many_arguments)]
    pub fn for_region(
        info: &CaptureInfo,
        origin: (i32, i32),
        logical_size: (u32, u32),
        stream_size: (u32, u32),
        image_size: (u32, u32),
        format: ImageFormat,
        elapsed: Duration,
    ) -> Self {
        Self {
            origin,
            logical_size,
            stream_size,
            scale_factor: info.scale_factor,
            image_size,
            image_to_logical_scale: image_to_logical_scale(
                stream_size,
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
    /// Where to move the pointer, as `[x, y]` in logical desktop coordinates.
    pub coordinate: [f64; 2],
}

/// The arguments the click tools accept.
///
/// One type for `left_click`, `right_click`, `middle_click`, `double_click`, and
/// `triple_click`: they differ only in which button and how many times, and
/// naming them separately — as the computer-use tools do — is what makes them
/// readable in a transcript.
#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct ClickArgs {
    /// Where to click, as `[x, y]` in logical desktop coordinates. Omitted clicks
    /// wherever the pointer already is.
    #[serde(default)]
    pub coordinate: Option<[f64; 2]>,

    /// Modifier keys to hold for the click: `shift`, `ctrl`, `alt`, `super`, or a
    /// `+`-joined combination such as `ctrl+shift`.
    #[serde(default)]
    pub modifiers: Option<String>,
}

/// The arguments `left_click_drag` accepts.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct DragArgs {
    /// Where the drag starts, as `[x, y]` in logical desktop coordinates.
    pub start_coordinate: [f64; 2],

    /// Where the drag ends, as `[x, y]` in logical desktop coordinates.
    pub coordinate: [f64; 2],

    /// Modifier keys to hold for the drag.
    #[serde(default)]
    pub modifiers: Option<String>,
}

/// The direction `scroll` moves the view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ScrollDirection {
    /// Towards the top of the content.
    Up,
    /// Towards the bottom of the content.
    Down,
    /// Towards the left of the content.
    Left,
    /// Towards the right of the content.
    Right,
}

impl ScrollDirection {
    /// The axis to scroll along and the signed amount in wheel steps.
    ///
    /// The backend keeps the wheel's sign convention — negative is up or left —
    /// so the direction turns into that sign here and a caller never writes a
    /// negative number to mean "down".
    pub fn axis_and_amount(self, amount: f64) -> (Axis, f64) {
        match self {
            Self::Up => (Axis::Vertical, -amount),
            Self::Down => (Axis::Vertical, amount),
            Self::Left => (Axis::Horizontal, -amount),
            Self::Right => (Axis::Horizontal, amount),
        }
    }

    /// The direction's name, as the tools' replies spell it.
    pub fn name(self) -> &'static str {
        match self {
            Self::Up => "up",
            Self::Down => "down",
            Self::Left => "left",
            Self::Right => "right",
        }
    }
}

/// The arguments `scroll` accepts.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ScrollArgs {
    /// Which way to scroll.
    pub scroll_direction: ScrollDirection,

    /// How many wheel steps to scroll, as a positive number.
    pub scroll_amount: f64,

    /// Where to scroll, as `[x, y]` in logical desktop coordinates. Omitted
    /// scrolls wherever the pointer already is.
    #[serde(default)]
    pub coordinate: Option<[f64; 2]>,

    /// Modifier keys to hold for the scroll.
    #[serde(default)]
    pub modifiers: Option<String>,
}

/// The arguments `type` accepts.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct TypeArgs {
    /// The text to type into whatever has focus.
    pub text: String,
}

/// The arguments `key` accepts.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct KeyArgs {
    /// The key or combination to press, such as `ctrl+shift+t`, `enter`, or
    /// `super`. Modifiers are `ctrl`, `shift`, `alt`, and `super`.
    pub text: String,

    /// How many times to press it, 1 to 100. Defaults to once.
    #[serde(default)]
    pub repeat: Option<u8>,
}

/// The arguments `hold_key` accepts.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct HoldKeyArgs {
    /// The key or combination to hold, such as `down` or `shift`.
    pub text: String,

    /// How long to hold it, in seconds, up to 300.
    pub duration: f64,
}

/// The arguments `zoom` accepts.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ZoomArgs {
    /// The region to capture, as `[x0, y0, x1, y1]` — the top-left and
    /// bottom-right corners in logical desktop coordinates.
    pub region: [f64; 4],

    /// Long-edge cap for the returned image, as in `screenshot`.
    #[serde(default)]
    pub max_dimension: Option<u32>,

    /// Image format, as in `screenshot`.
    #[serde(default)]
    pub format: Option<ScreenshotFormat>,

    /// JPEG quality, 1 to 100, as in `screenshot`.
    #[serde(default)]
    pub quality: Option<u8>,
}

/// The arguments `wait` accepts.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct WaitArgs {
    /// How long to wait, in seconds, up to 300.
    pub duration: f64,
}

/// The arguments `wait_for_change` accepts.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct WaitForChangeArgs {
    /// How long to wait for a change, in seconds, up to 300.
    pub timeout: f64,

    /// The fraction of sampled pixels that must differ before the screen counts
    /// as changed. Defaults to a value above a blinking caret's footprint.
    #[serde(default)]
    pub min_change: Option<f64>,
}

/// The longest `duration` or `timeout` a timing tool accepts, in seconds.
pub const MAX_WAIT_SECONDS: f64 = 300.0;

/// How long `wait_for_change` sleeps between checks.
pub const WAIT_POLL_INTERVAL: Duration = Duration::from_millis(100);

/// The fraction of sampled pixels that must differ before `wait_for_change`
/// calls the screen changed.
///
/// Above a blinking text caret's footprint, so a caret does not read as a
/// change; a panel or a menu opening is many times larger.
pub const DEFAULT_MIN_CHANGE: f64 = 0.001;

/// How far a colour channel must move before a pixel counts as changed.
const CHANGE_CHANNEL_TOLERANCE: u8 = 8;

/// The most pixels `changed_fraction` samples, so the poll stays cheap.
const CHANGE_SAMPLES: usize = 100_000;

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

/// A seconds argument as a `Duration`, checked and capped.
///
/// Shared by `wait` and `hold_key`, which both take a number of seconds. A
/// negative or non-finite one is a caller mistake worth naming rather than
/// quietly treating as zero.
pub fn seconds_to_duration(seconds: f64, name: &str) -> Result<Duration, String> {
    if !seconds.is_finite() || seconds < 0.0 {
        return Err(format!(
            "`{name}` must be a number of seconds, not {seconds}"
        ));
    }
    Ok(Duration::from_secs_f64(seconds.min(MAX_WAIT_SECONDS)))
}

/// Copy the `width`x`height` rectangle at `(x, y)` out of a frame.
///
/// The coordinates are in the frame's own pixels, so the caller converts from
/// logical desktop coordinates first. This is what `zoom` is built on: a region
/// keeps its resolution where the whole desktop would have been reduced.
pub fn crop(frame: &Frame, x: u32, y: u32, width: u32, height: u32) -> Result<Frame, Error> {
    let fits = width > 0
        && height > 0
        && x.checked_add(width)
            .is_some_and(|right| right <= frame.width)
        && y.checked_add(height)
            .is_some_and(|bottom| bottom <= frame.height);
    if !fits || !frame.is_well_formed() {
        return Err(Error::Capture(format!(
            "the region ({x}, {y}) {width}x{height} is not inside the {}x{} frame",
            frame.width, frame.height
        )));
    }

    let source_stride = frame.width as usize * Frame::BYTES_PER_PIXEL;
    let row_bytes = width as usize * Frame::BYTES_PER_PIXEL;
    let mut rgb = Vec::with_capacity(row_bytes * height as usize);
    for row in y..y + height {
        let start = row as usize * source_stride + x as usize * Frame::BYTES_PER_PIXEL;
        rgb.extend_from_slice(&frame.rgb[start..start + row_bytes]);
    }

    Ok(Frame { width, height, rgb })
}

/// The fraction of sampled pixels that differ between two frames.
///
/// Sampled rather than exhaustive because this runs inside a polling loop: a
/// change worth waking for shows up across a fraction of the pixels, so examining
/// a bounded number of them costs little and decides the same question. Frames of
/// different sizes count as entirely changed.
pub fn changed_fraction(before: &Frame, after: &Frame) -> f64 {
    if (before.width, before.height) != (after.width, after.height)
        || !before.is_well_formed()
        || !after.is_well_formed()
    {
        return 1.0;
    }

    let pixels = before.width as usize * before.height as usize;
    if pixels == 0 {
        return 0.0;
    }

    let step = pixels.div_ceil(CHANGE_SAMPLES).max(1);
    let mut sampled = 0usize;
    let mut changed = 0usize;
    let mut index = 0usize;
    while index < pixels {
        let offset = index * Frame::BYTES_PER_PIXEL;
        let differs = (0..Frame::BYTES_PER_PIXEL).any(|channel| {
            before.rgb[offset + channel].abs_diff(after.rgb[offset + channel])
                > CHANGE_CHANNEL_TOLERANCE
        });
        if differs {
            changed += 1;
        }
        sampled += 1;
        index += step;
    }

    if sampled == 0 {
        0.0
    } else {
        changed as f64 / sampled as f64
    }
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
        assert_eq!(to_base64(b"computer-use"), "Y29tcHV0ZXItdXNl");
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

    #[test]
    fn region_metadata_maps_a_zoomed_pixel_back_to_the_desktop() {
        let info = CaptureInfo {
            logical_size: (1000, 800),
            stream_size: (2000, 1600),
            origin: (0, 0),
            scale_factor: 2.0,
            pointer: Some((10.0, 20.0)),
            input_available: true,
        };

        let metadata = ScreenshotMetadata::for_region(
            &info,
            (50, 25),
            (200, 100),
            (400, 200),
            (200, 100),
            ImageFormat::Png,
            Duration::from_millis(5),
        );

        assert_eq!(metadata.origin, (50, 25));
        assert_eq!(metadata.logical_size, (200, 100));
        assert_eq!(metadata.stream_size, (400, 200));
        assert_eq!(metadata.pointer, Some((10.0, 20.0)));
        // 400 stream pixels reduced to 200 image pixels, over a scale factor of 2.
        assert!((metadata.image_to_logical_scale - 1.0).abs() < 1e-9);
        assert!((200.0 * metadata.image_to_logical_scale - 200.0).abs() < 1e-6);
    }

    #[test]
    fn cropping_takes_the_requested_rectangle() {
        let source = Frame {
            width: 3,
            height: 2,
            rgb: vec![
                1, 2, 3, 4, 5, 6, 7, 8, 9, //
                10, 11, 12, 13, 14, 15, 16, 17, 18,
            ],
        };

        let region = crop(&source, 1, 0, 2, 2).expect("a crop");
        assert_eq!((region.width, region.height), (2, 2));
        assert_eq!(region.rgb, vec![4, 5, 6, 7, 8, 9, 13, 14, 15, 16, 17, 18]);
    }

    #[test]
    fn a_region_outside_the_frame_is_refused() {
        let source = frame(4, 4);
        assert!(crop(&source, 3, 0, 2, 1).is_err(), "past the right edge");
        assert!(crop(&source, 0, 3, 1, 2).is_err(), "past the bottom edge");
        assert!(crop(&source, 0, 0, 0, 1).is_err(), "an empty region");
        assert!(crop(&source, 0, 0, 4, 4).is_ok(), "the whole frame");
    }

    #[test]
    fn identical_frames_have_not_changed() {
        let before = frame(32, 32);
        assert_eq!(changed_fraction(&before, &before.clone()), 0.0);
    }

    #[test]
    fn a_frame_that_differs_everywhere_has_changed_completely() {
        let before = frame(32, 32);
        let after = Frame {
            width: 32,
            height: 32,
            rgb: vec![0xff; Frame::expected_len(32, 32)],
        };
        assert_eq!(changed_fraction(&before, &after), 1.0);
    }

    #[test]
    fn a_difference_below_the_channel_tolerance_is_not_a_change() {
        let before = frame(64, 64);
        let mut after = before.clone();
        // Every channel shifts by the tolerance and no more, the size of change a
        // caret's antialiasing makes; none of it counts.
        for byte in after.rgb.iter_mut() {
            *byte = byte.saturating_add(CHANGE_CHANNEL_TOLERANCE);
        }
        assert_eq!(changed_fraction(&before, &after), 0.0);
    }

    #[test]
    fn frames_of_different_sizes_count_as_changed() {
        assert_eq!(changed_fraction(&frame(8, 8), &frame(8, 9)), 1.0);
    }

    #[test]
    fn a_seconds_argument_is_capped_and_checked() {
        assert_eq!(
            seconds_to_duration(1.5, "duration").expect("a duration"),
            Duration::from_secs_f64(1.5)
        );
        assert_eq!(
            seconds_to_duration(10_000.0, "duration").expect("a duration"),
            Duration::from_secs_f64(MAX_WAIT_SECONDS)
        );
        assert!(seconds_to_duration(-1.0, "duration").is_err());
        assert!(seconds_to_duration(f64::NAN, "duration").is_err());
    }

    #[test]
    fn a_scroll_direction_carries_its_own_sign() {
        assert_eq!(
            ScrollDirection::Up.axis_and_amount(3.0),
            (Axis::Vertical, -3.0)
        );
        assert_eq!(
            ScrollDirection::Down.axis_and_amount(3.0),
            (Axis::Vertical, 3.0)
        );
        assert_eq!(
            ScrollDirection::Left.axis_and_amount(2.0),
            (Axis::Horizontal, -2.0)
        );
        assert_eq!(
            ScrollDirection::Right.axis_and_amount(2.0),
            (Axis::Horizontal, 2.0)
        );
        assert_eq!(ScrollDirection::Right.name(), "right");
        assert_eq!(ScrollDirection::Left.name(), "left");
    }
}
