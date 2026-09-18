//! The command-line interface.

use clap::{Parser, ValueEnum};

/// The default long-edge cap applied to screenshots, in pixels.
///
/// This matches the long-edge limit common vision models apply, so a screenshot
/// is not downscaled twice, once by us and once by the client.
pub const DEFAULT_MAX_DIMENSION: u32 = 1568;

/// An MCP server that lets an AI agent observe and control a Wayland Linux
/// desktop.
#[derive(Debug, Clone, Parser)]
#[command(name = "desktop-use", version, about, long_about = None)]
pub struct Cli {
    /// Open a window showing the frames the model sees.
    ///
    /// Makes the session observable, so the desktop is never driven invisibly.
    #[arg(long)]
    pub preview: bool,

    /// Long-edge cap, in pixels, applied to screenshots before they are encoded.
    #[arg(long, value_name = "PIXELS", default_value_t = DEFAULT_MAX_DIMENSION)]
    pub max_dimension: u32,

    /// Image format used for screenshots.
    #[arg(long, value_enum, default_value_t = ImageFormat::Png)]
    pub format: ImageFormat,
}

/// The image format a screenshot is encoded in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ImageFormat {
    /// Lossless. The default, because an agent reading text off a UI needs it.
    Png,
    /// Lossy and smaller, for when cost matters more than fidelity.
    Jpeg,
}

impl ImageFormat {
    /// The MIME type this format is returned as in MCP image content.
    pub fn mime_type(self) -> &'static str {
        match self {
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
        }
    }
}

impl Cli {
    /// The defaults the MCP tools fall back on when a caller omits an argument.
    ///
    /// These options exist on the command line so that an operator can set them
    /// once for a session instead of repeating them on every call.
    pub fn tool_defaults(&self) -> crate::tools::ToolDefaults {
        crate::tools::ToolDefaults {
            max_dimension: self.max_dimension,
            format: self.format,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_the_documented_values() {
        let cli = Cli::parse_from(["desktop-use"]);
        assert!(!cli.preview);
        assert_eq!(cli.max_dimension, DEFAULT_MAX_DIMENSION);
        assert_eq!(cli.format, ImageFormat::Png);
    }

    #[test]
    fn flags_are_accepted() {
        let cli = Cli::parse_from([
            "desktop-use",
            "--preview",
            "--max-dimension",
            "512",
            "--format",
            "jpeg",
        ]);
        assert!(cli.preview);
        assert_eq!(cli.max_dimension, 512);
        assert_eq!(cli.format, ImageFormat::Jpeg);
        assert_eq!(cli.format.mime_type(), "image/jpeg");
    }
}
