//! Take a reference screenshot through the portal's `Screenshot` interface.
//!
//! This is a verification aid, not part of the server. The portal's screenshot
//! path has GNOME write the PNG itself, through a completely different code path
//! from our PipeWire capture, which makes it an independent reference for
//! checking that our pixel conversion is right — in particular that the colour
//! channels are not swapped, which no amount of self-consistency can reveal.
//!
//! GNOME saves the file under `~/Pictures/Screenshots` before handing back the
//! URI; this copies it to the requested path.
//!
//! ```sh
//! cargo run --example reference_screenshot -- /tmp/reference.png
//! ```

use std::process::ExitCode;

use ashpd::desktop::screenshot::Screenshot;

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
    let destination = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "reference.png".to_string());

    // Non-interactive: no chooser dialog, just the whole desktop.
    let request = Screenshot::request().interactive(false).send().await?;
    let screenshot = request.response()?;

    let uri = screenshot.uri();
    let source = uri
        .to_file_path()
        .map_err(|()| format!("the portal returned a URI that is not a file: {uri}"))?;

    std::fs::copy(&source, &destination)?;
    println!("reference: {}", source.display());
    println!("copied to {destination}");

    Ok(())
}
