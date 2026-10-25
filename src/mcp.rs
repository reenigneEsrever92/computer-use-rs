//! The MCP server: the boundary an agent talks to.
//!
//! The tools here are thin. Everything they do is call the backend and translate
//! the result into what MCP expects, with the arithmetic living in
//! [`crate::tools`] where it can be tested without a session, and the composition
//! of input events living in [`crate::backend::input`] where it can be tested
//! against a mock backend.

use std::sync::Arc;
use std::time::Instant;

use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, Content, Implementation, ServerCapabilities, ServerInfo};
use rmcp::transport::stdio;
use rmcp::{ServerHandler, ServiceExt, tool, tool_handler, tool_router};

use crate::backend::input::{self, Button, KeyState};
use crate::backend::keys::{self, Keysym};
use crate::backend::{Backends, CaptureBackend, InputBackend};
use crate::cli::ImageFormat;
use crate::error::Error;
use crate::tools::{
    ClickArgs, DEFAULT_JPEG_QUALITY, DEFAULT_MIN_CHANGE, DisplayInfo, DragArgs, HoldKeyArgs,
    KeyArgs, MouseMoveArgs, ScreenshotArgs, ScreenshotMetadata, ScrollArgs, ToolDefaults, TypeArgs,
    WAIT_POLL_INTERVAL, WaitArgs, WaitForChangeArgs, ZoomArgs, changed_fraction, crop, encode,
    fit_within, resize, seconds_to_duration, to_base64,
};

/// The MCP server.
#[derive(Clone)]
pub struct DesktopUseServer {
    /// The platform backends, shared with the preview window when one is open.
    backends: Arc<Backends>,
    /// What the tools fall back on when a caller omits an argument.
    defaults: ToolDefaults,
}

impl DesktopUseServer {
    /// Create a server over the given backends and defaults.
    pub fn new(backends: Arc<Backends>, defaults: ToolDefaults) -> Self {
        Self { backends, defaults }
    }

    /// The capture backend, or the reason there is none.
    ///
    /// A tool error rather than a panic: an agent should be told what is missing,
    /// not handed a protocol fault.
    fn capture(&self) -> Result<&dyn CaptureBackend, String> {
        self.backends.capture.as_deref().ok_or_else(|| {
            Error::BackendUnavailable {
                backend: "capture",
                detail: "this build has no way to capture the desktop",
            }
            .to_string()
        })
    }

    /// The input backend, or the reason there is none.
    fn input(&self) -> Result<&dyn InputBackend, String> {
        self.backends.input.as_deref().ok_or_else(|| {
            Error::BackendUnavailable {
                backend: "input",
                detail: "this build has no way to control the desktop",
            }
            .to_string()
        })
    }

    /// Move when a position is given, then click, holding any modifiers.
    ///
    /// Shared by the five named click tools, which differ only in button and
    /// count. The move comes before the modifiers so that a shift-click lands
    /// where it was asked to rather than where the pointer happened to be.
    async fn click_with(
        &self,
        args: ClickArgs,
        button: Button,
        count: u8,
    ) -> Result<String, String> {
        let modifiers = parse_modifiers(&args.modifiers)?;
        let input = self.input()?;

        if let Some([x, y]) = args.coordinate {
            input
                .pointer_move(x, y)
                .await
                .map_err(|error| error.to_string())?;
        }

        input::click(input, button, count, &modifiers)
            .await
            .map_err(|error| error.to_string())?;

        let times = if count > 1 {
            format!(" {count} times")
        } else {
            String::new()
        };
        Ok(format!("clicked {}{times}", button_name(button)))
    }
}

#[tool_router]
impl DesktopUseServer {
    /// Capture the desktop and return it as an image.
    ///
    /// The image is accompanied by the metadata needed to act on what it shows:
    /// multiply a position in the returned image by `image_to_logical_scale` to
    /// get the logical desktop coordinates the input tools accept.
    #[tool]
    async fn screenshot(
        &self,
        Parameters(args): Parameters<ScreenshotArgs>,
    ) -> Result<CallToolResult, String> {
        let started = Instant::now();
        let capture = self.capture()?;

        // `connect` is idempotent and is what prompts for consent the first time.
        let info = capture.connect().await.map_err(|error| error.to_string())?;
        let frame = capture
            .capture_frame()
            .await
            .map_err(|error| error.to_string())?;
        let captured = started.elapsed();

        let format = args
            .format
            .map(ImageFormat::from)
            .unwrap_or(self.defaults.format);
        let cap = args.max_dimension.unwrap_or(self.defaults.max_dimension);
        let quality = args.quality.unwrap_or(DEFAULT_JPEG_QUALITY).clamp(1, 100);

        let size = fit_within(frame.width, frame.height, cap);
        let reduced = resize(&frame, size).map_err(|error| error.to_string())?;
        let reduced_at = started.elapsed();

        let bytes = encode(&reduced, format, quality).map_err(|error| error.to_string())?;
        let encoded_at = started.elapsed();

        // Timed per stage, so a regression says which stage regressed rather than
        // only that the whole call got slower.
        tracing::debug!(
            "screenshot: capture {captured:?}, resize {:?}, encode {:?}, total {encoded_at:?}",
            reduced_at - captured,
            encoded_at - reduced_at,
        );

        let metadata =
            ScreenshotMetadata::new(&info, (frame.width, frame.height), size, format, encoded_at);
        let metadata =
            serde_json::to_string_pretty(&metadata).map_err(|error| error.to_string())?;

        Ok(CallToolResult::success(vec![
            Content::image(to_base64(&bytes), format.mime_type()),
            Content::text(metadata),
        ]))
    }

    /// Capture one region of the desktop at full resolution.
    ///
    /// Use this when a full `screenshot` leaves text illegible: `region` spends
    /// the resolution budget on a small area instead of the whole screen. The
    /// region is given in logical desktop coordinates, like every other tool, so
    /// a position read from a screenshot's metadata can be used here unchanged.
    #[tool]
    async fn zoom(&self, Parameters(args): Parameters<ZoomArgs>) -> Result<CallToolResult, String> {
        let started = Instant::now();
        let capture = self.capture()?;
        let info = capture.connect().await.map_err(|error| error.to_string())?;
        let frame = capture
            .capture_frame()
            .await
            .map_err(|error| error.to_string())?;

        let [x0, y0, x1, y1] = args.region;
        if !(x1 > x0 && y1 > y0) {
            return Err(format!(
                "the region {:?} is empty; give [x0, y0, x1, y1] with x1 > x0 and y1 > y0",
                args.region
            ));
        }

        // Logical coordinates to the frame's pixels, then clamped: a position a
        // half-pixel past the edge should get the pixels that exist rather than
        // an error about the overhang.
        let (sx0, sy0) = info.logical_to_stream((x0, y0));
        let (sx1, sy1) = info.logical_to_stream((x1, y1));
        let left = sx0.floor().max(0.0) as u32;
        let top = sy0.floor().max(0.0) as u32;
        let right = (sx1.ceil().max(0.0) as u32).min(frame.width);
        let bottom = (sy1.ceil().max(0.0) as u32).min(frame.height);
        if right <= left || bottom <= top {
            return Err(format!(
                "the region {:?} does not overlap the captured desktop",
                args.region
            ));
        }

        let region =
            crop(&frame, left, top, right - left, bottom - top).map_err(|e| e.to_string())?;

        let format = args
            .format
            .map(ImageFormat::from)
            .unwrap_or(self.defaults.format);
        let cap = args.max_dimension.unwrap_or(self.defaults.max_dimension);
        let quality = args.quality.unwrap_or(DEFAULT_JPEG_QUALITY).clamp(1, 100);

        let size = fit_within(region.width, region.height, cap);
        let reduced = resize(&region, size).map_err(|error| error.to_string())?;
        let bytes = encode(&reduced, format, quality).map_err(|error| error.to_string())?;

        // The region's own geometry in both spaces, so a coordinate read off the
        // zoomed image maps back onto the desktop the way a screenshot's does.
        let scale = info.scale_factor;
        let origin = (
            info.origin.0 + (left as f64 / scale).round() as i32,
            info.origin.1 + (top as f64 / scale).round() as i32,
        );
        let logical_size = (
            ((right - left) as f64 / scale).round().max(1.0) as u32,
            ((bottom - top) as f64 / scale).round().max(1.0) as u32,
        );

        let metadata = ScreenshotMetadata::for_region(
            &info,
            origin,
            logical_size,
            (right - left, bottom - top),
            size,
            format,
            started.elapsed(),
        );
        let metadata =
            serde_json::to_string_pretty(&metadata).map_err(|error| error.to_string())?;

        Ok(CallToolResult::success(vec![
            Content::image(to_base64(&bytes), format.mime_type()),
            Content::text(metadata),
        ]))
    }

    /// Report the display geometry, and whether input can be injected here.
    ///
    /// Use this before acting: on a session where `input_available` is false, the
    /// input tools cannot work, and knowing that is better than discovering it
    /// through a failed click.
    #[tool]
    async fn get_display_info(&self) -> Result<String, String> {
        let capture = self.capture()?;
        let info = capture.connect().await.map_err(|error| error.to_string())?;

        let report = DisplayInfo::new(&info, capture.name());
        serde_json::to_string_pretty(&report).map_err(|error| error.to_string())
    }

    /// Report where the pointer is, in logical desktop coordinates.
    ///
    /// The position is the last one this server commanded. The portal's
    /// RemoteDesktop interface does not report where the pointer actually is, so
    /// before anything has moved it this says the position is unknown rather than
    /// guessing.
    #[tool]
    async fn cursor_position(&self) -> Result<String, String> {
        let capture = self.capture()?;
        let info = capture.connect().await.map_err(|error| error.to_string())?;

        Ok(match info.pointer {
            Some((x, y)) => format!("the pointer is at ({x}, {y})"),
            None => "the pointer position is not known yet; it is reported once this server has \
                     moved the pointer"
                .to_string(),
        })
    }

    /// Move the pointer to a position in logical desktop coordinates.
    ///
    /// Coordinates are the ones a screenshot's metadata reports, so a position
    /// read off an image can be passed here unchanged.
    #[tool]
    async fn mouse_move(
        &self,
        Parameters(args): Parameters<MouseMoveArgs>,
    ) -> Result<String, String> {
        let [x, y] = args.coordinate;
        self.input()?
            .pointer_move(x, y)
            .await
            .map_err(|error| error.to_string())?;
        Ok(format!("the pointer is at ({x}, {y})"))
    }

    /// Click the primary button, optionally at a position and with modifiers.
    ///
    /// `modifiers` holds `shift`, `ctrl`, `alt`, or `super` for the click only,
    /// which is how a range or a multi-selection is extended.
    #[tool]
    async fn left_click(&self, Parameters(args): Parameters<ClickArgs>) -> Result<String, String> {
        self.click_with(args, Button::Left, 1).await
    }

    /// Click the secondary button, optionally at a position and with modifiers.
    #[tool]
    async fn right_click(&self, Parameters(args): Parameters<ClickArgs>) -> Result<String, String> {
        self.click_with(args, Button::Right, 1).await
    }

    /// Click the middle button, optionally at a position and with modifiers.
    #[tool]
    async fn middle_click(
        &self,
        Parameters(args): Parameters<ClickArgs>,
    ) -> Result<String, String> {
        self.click_with(args, Button::Middle, 1).await
    }

    /// Double-click the primary button, optionally at a position.
    #[tool]
    async fn double_click(
        &self,
        Parameters(args): Parameters<ClickArgs>,
    ) -> Result<String, String> {
        self.click_with(args, Button::Left, 2).await
    }

    /// Triple-click the primary button, optionally at a position.
    #[tool]
    async fn triple_click(
        &self,
        Parameters(args): Parameters<ClickArgs>,
    ) -> Result<String, String> {
        self.click_with(args, Button::Left, 3).await
    }

    /// Press the primary button at `start_coordinate`, drag to `coordinate`, and
    /// release.
    ///
    /// The motion is interpolated, so applications that wait for movement before
    /// they recognise a drag still see one. For a drag that needs more than one
    /// gesture, use `left_mouse_down` and `left_mouse_up` around `mouse_move`.
    #[tool]
    async fn left_click_drag(
        &self,
        Parameters(args): Parameters<DragArgs>,
    ) -> Result<String, String> {
        let modifiers = parse_modifiers(&args.modifiers)?;
        let [x0, y0] = args.start_coordinate;
        let [x1, y1] = args.coordinate;

        input::drag(self.input()?, (x0, y0), (x1, y1), &modifiers)
            .await
            .map_err(|error| error.to_string())?;

        Ok(format!("dragged from ({x0}, {y0}) to ({x1}, {y1})"))
    }

    /// Press the primary button and leave it down.
    ///
    /// Move the pointer first with `mouse_move`, then release with
    /// `left_mouse_up`. For a single drag, `left_click_drag` is simpler.
    #[tool]
    async fn left_mouse_down(&self) -> Result<String, String> {
        self.input()?
            .pointer_button(Button::Left, KeyState::Pressed)
            .await
            .map_err(|error| error.to_string())?;
        Ok("the primary button is down".to_string())
    }

    /// Release the primary button.
    #[tool]
    async fn left_mouse_up(&self) -> Result<String, String> {
        self.input()?
            .pointer_button(Button::Left, KeyState::Released)
            .await
            .map_err(|error| error.to_string())?;
        Ok("the primary button is up".to_string())
    }

    /// Scroll the view, optionally at a position and with modifiers.
    ///
    /// `scroll_amount` is a positive number of wheel steps; `scroll_direction`
    /// says which way, so a caller never writes a negative amount to mean "down".
    #[tool]
    async fn scroll(&self, Parameters(args): Parameters<ScrollArgs>) -> Result<String, String> {
        let modifiers = parse_modifiers(&args.modifiers)?;
        let (axis, amount) = args.scroll_direction.axis_and_amount(args.scroll_amount);
        let at = args.coordinate.map(|[x, y]| (x, y));

        input::scroll(self.input()?, axis, amount, at, &modifiers)
            .await
            .map_err(|error| error.to_string())?;

        Ok(format!(
            "scrolled {} by {}",
            args.scroll_direction.name(),
            args.scroll_amount
        ))
    }

    /// Type text into whatever currently has focus.
    ///
    /// For shortcuts, use `key` instead: typing a string is layout- and
    /// Unicode-sensitive, pressing a shortcut is not, and one tool trying to be
    /// both would do neither well.
    ///
    /// The method is `type_text` because `type` is a Rust keyword; the attribute
    /// gives it the wire name the tool surface uses.
    #[tool(name = "type")]
    async fn type_text(&self, Parameters(args): Parameters<TypeArgs>) -> Result<String, String> {
        let characters = args.text.chars().count();
        input::type_text(self.input()?, &args.text)
            .await
            .map_err(|error| error.to_string())?;
        Ok(format!("typed {characters} character(s)"))
    }

    /// Press a key or a combination, such as `enter` or `ctrl+shift+t`.
    ///
    /// `repeat` presses it that many times, which is what an agent wants for
    /// stepping through a list or form with `tab` or an arrow key.
    #[tool]
    async fn key(&self, Parameters(args): Parameters<KeyArgs>) -> Result<String, String> {
        let combination = keys::parse_combination(&args.text)?;
        let repeat = args.repeat.unwrap_or(1).clamp(1, 100);

        input::press_combination(self.input()?, &combination, repeat)
            .await
            .map_err(|error| error.to_string())?;

        Ok(format!("pressed {} {repeat} time(s)", args.text))
    }

    /// Hold a key or a combination down for a duration, in seconds.
    ///
    /// Holding is what drives key auto-repeat, so this is how an arrow key scrolls
    /// a list or moves through a document continuously.
    #[tool]
    async fn hold_key(&self, Parameters(args): Parameters<HoldKeyArgs>) -> Result<String, String> {
        let combination = keys::parse_combination(&args.text)?;
        let duration = seconds_to_duration(args.duration, "duration")?;

        input::hold_keys(self.input()?, &combination, duration)
            .await
            .map_err(|error| error.to_string())?;

        Ok(format!("held {} for {duration:?}", args.text))
    }

    /// Pause before the next action, for an application that needs time.
    ///
    /// When the thing being waited for is visible, `wait_for_change` is better: it
    /// returns as soon as the screen reacts instead of after a guessed delay.
    #[tool]
    async fn wait(&self, Parameters(args): Parameters<WaitArgs>) -> Result<String, String> {
        let duration = seconds_to_duration(args.duration, "duration")?;
        tokio::time::sleep(duration).await;
        Ok(format!("waited {duration:?}"))
    }

    /// Wait until the screen changes, or until the timeout passes.
    ///
    /// Returns as soon as the change lands, which removes the race that a fixed
    /// `wait` leaves. `min_change` is the fraction of pixels that must differ;
    /// the default is above a blinking caret's footprint, so a caret does not
    /// count as a change.
    #[tool]
    async fn wait_for_change(
        &self,
        Parameters(args): Parameters<WaitForChangeArgs>,
    ) -> Result<String, String> {
        let timeout = seconds_to_duration(args.timeout, "timeout")?;
        let min_change = args
            .min_change
            .unwrap_or(DEFAULT_MIN_CHANGE)
            .clamp(0.0, 1.0);

        let capture = self.capture()?;
        capture.connect().await.map_err(|error| error.to_string())?;
        let baseline = capture
            .capture_frame()
            .await
            .map_err(|error| error.to_string())?;

        let started = Instant::now();
        loop {
            if started.elapsed() >= timeout {
                return Ok(format!("the screen did not change within {timeout:?}"));
            }

            tokio::time::sleep(WAIT_POLL_INTERVAL).await;
            let current = capture
                .capture_frame()
                .await
                .map_err(|error| error.to_string())?;

            // The frame source hands back the same frame until the compositor
            // delivers a new one, so an unchanged screen costs a pointer compare
            // rather than a pixel walk.
            if Arc::ptr_eq(&current, &baseline) {
                continue;
            }

            let changed = changed_fraction(&baseline, &current);
            if changed >= min_change {
                return Ok(format!(
                    "the screen changed after {:?} ({:.1}% of sampled pixels differ)",
                    started.elapsed(),
                    changed * 100.0
                ));
            }
        }
    }
}

#[tool_handler]
impl ServerHandler for DesktopUseServer {
    /// Identify this server as `computer-use`.
    ///
    /// `#[tool_handler]` would otherwise generate this from
    /// `Implementation::from_build_env()`, which reads *rmcp's* build
    /// environment and therefore announces the SDK as the server.
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build()).with_server_info(
            Implementation::new(env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION")),
        )
    }
}

/// The modifier keysyms named by a tool argument.
///
/// Absent means none; a name that is not a modifier is an error, because dropping
/// it would quietly send an unmodified action where a modified one was asked for.
fn parse_modifiers(text: &Option<String>) -> Result<Vec<Keysym>, String> {
    match text {
        None => Ok(Vec::new()),
        Some(text) => keys::parse_modifiers(text),
    }
}

/// A button's name, as the tools' replies spell it.
fn button_name(button: Button) -> &'static str {
    match button {
        Button::Left => "left",
        Button::Middle => "middle",
        Button::Right => "right",
    }
}

/// Serve MCP over stdio until the client disconnects.
///
/// Returns once the transport closes, which is what happens when the client
/// closes stdin. A clean close is a normal exit, not an error.
pub async fn serve(backends: Arc<Backends>, defaults: ToolDefaults) -> Result<(), Error> {
    let running = DesktopUseServer::new(backends, defaults)
        .serve(stdio())
        .await
        .map_err(|error| Error::Mcp(error.to_string()))?;

    running
        .waiting()
        .await
        .map_err(|error| Error::Mcp(error.to_string()))?;

    Ok(())
}
