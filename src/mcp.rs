//! The MCP server: the boundary an agent talks to.
//!
//! The tools here are thin. Everything they do is call the backend and translate
//! the result into what MCP expects, with the arithmetic living in
//! [`crate::tools`] where it can be tested without a session.

use std::sync::Arc;
use std::time::Instant;

use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, Content, Implementation, ServerCapabilities, ServerInfo};
use rmcp::transport::stdio;
use rmcp::{ServerHandler, ServiceExt, tool, tool_handler, tool_router};

use crate::backend::{Backends, CaptureBackend, InputBackend};
use crate::cli::ImageFormat;
use crate::error::Error;
use crate::tools::{
    AxisArg, ButtonArg, DEFAULT_JPEG_QUALITY, DisplayInfo, KeyPressArgs, MouseClickArgs,
    MouseMoveArgs, MouseScrollArgs, ScreenshotArgs, ScreenshotMetadata, ToolDefaults, TypeTextArgs,
    encode, fit_within, resize, to_base64,
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

    /// Move the pointer to a position in logical desktop coordinates.
    ///
    /// Coordinates are the ones a screenshot's metadata reports, so a position
    /// read off an image can be passed here unchanged.
    #[tool]
    async fn mouse_move(
        &self,
        Parameters(args): Parameters<MouseMoveArgs>,
    ) -> Result<String, String> {
        self.input()?
            .pointer_move(args.x, args.y)
            .await
            .map_err(|error| error.to_string())?;
        Ok(format!("the pointer is at ({}, {})", args.x, args.y))
    }

    /// Click a pointer button, optionally moving there first.
    #[tool]
    async fn mouse_click(
        &self,
        Parameters(args): Parameters<MouseClickArgs>,
    ) -> Result<String, String> {
        let input = self.input()?;

        if let (Some(x), Some(y)) = (args.x, args.y) {
            input
                .pointer_move(x, y)
                .await
                .map_err(|error| error.to_string())?;
        }

        let button = args.button.unwrap_or(ButtonArg::Left);
        let count = args.count.unwrap_or(1);
        input
            .pointer_button(button.into(), count)
            .await
            .map_err(|error| error.to_string())?;

        Ok(format!("clicked {button:?} {count} time(s)"))
    }

    /// Scroll along an axis, in whole wheel steps.
    #[tool]
    async fn mouse_scroll(
        &self,
        Parameters(args): Parameters<MouseScrollArgs>,
    ) -> Result<String, String> {
        let axis = args.axis.unwrap_or(AxisArg::Vertical);
        self.input()?
            .pointer_scroll(axis.into(), args.amount)
            .await
            .map_err(|error| error.to_string())?;
        Ok(format!("scrolled {axis:?} by {}", args.amount))
    }

    /// Type text into whatever currently has focus.
    ///
    /// For shortcuts, use `key_press` instead: typing a string and pressing a key
    /// combination are different operations, and one tool trying to be both would
    /// do neither well.
    #[tool]
    async fn type_text(
        &self,
        Parameters(args): Parameters<TypeTextArgs>,
    ) -> Result<String, String> {
        let characters = args.text.chars().count();
        self.input()?
            .type_text(args.text)
            .await
            .map_err(|error| error.to_string())?;
        Ok(format!("typed {characters} character(s)"))
    }

    /// Press a key combination, such as `ctrl+shift+t`.
    #[tool]
    async fn key_press(
        &self,
        Parameters(args): Parameters<KeyPressArgs>,
    ) -> Result<String, String> {
        self.input()?
            .key_press(args.combination.clone())
            .await
            .map_err(|error| error.to_string())?;
        Ok(format!("pressed {}", args.combination))
    }
}

#[tool_handler]
impl ServerHandler for DesktopUseServer {
    /// Identify this server as `desktop-use`.
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
