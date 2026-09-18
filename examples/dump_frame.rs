//! Capture one frame and write it out as a PNG.
//!
//! This exists so capture can be exercised and looked at without a window and
//! without an MCP client. One command shows the whole path: the consent prompt,
//! the portal session, the PipeWire stream, the pixel conversion, and the
//! resulting image.
//!
//! ```sh
//! cargo run --example dump_frame -- /tmp/frame.png
//! ```

use std::process::ExitCode;

use desktop_use::backend::resolve;

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "frame.png".to_string());

    desktop_use::logging::init();

    let backends = resolve();
    let capture = backends
        .capture
        .as_ref()
        .ok_or("no capture backend is available")?;
    println!("backend: {}", capture.name());

    // This is where the consent prompt appears on first use.
    let info = capture.connect().await?;
    println!(
        "logical: {}x{} at ({}, {})",
        info.logical_size.0, info.logical_size.1, info.origin.0, info.origin.1
    );
    println!(
        "stream:  {}x{} (scale {:.4})",
        info.stream_size.0, info.stream_size.1, info.scale_factor
    );
    println!("input available: {}", info.input_available);

    let frame = capture.capture_frame().await?;
    println!(
        "frame: {}x{} ({} bytes of RGB)",
        frame.width,
        frame.height,
        frame.rgb.len()
    );

    if !frame.is_well_formed() {
        return Err(format!(
            "the frame's pixel buffer does not match its dimensions: {} bytes for {}x{}",
            frame.rgb.len(),
            frame.width,
            frame.height
        )
        .into());
    }

    let image = image::RgbImage::from_raw(frame.width, frame.height, frame.rgb.to_vec())
        .ok_or("the frame could not be interpreted as an RGB image")?;
    image.save(&path)?;
    println!("wrote {path}");

    Ok(())
}
