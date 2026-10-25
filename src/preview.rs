//! The `--preview` window.
//!
//! A window showing the frames the model is looking at, so that the desktop is
//! never driven invisibly. It exists to be watched, not to be used: it takes no
//! input and offers no controls beyond its own frame.
//!
//! Compiled only when the `preview` feature is on, so a headless build carries no
//! GUI dependency at all.

use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::time::Duration;

use eframe::egui;

use crate::backend::Backends;
use crate::backend::capture::Frame;
use crate::error::Error;

/// How often the pump asks the capture backend for the newest frame.
///
/// The backend serves the most recent frame it has rather than blocking for a
/// new one, so this is a refresh rate rather than a latency. About 30 Hz matches
/// what a compositor delivers and is far below the cost of re-encoding.
const POLL_INTERVAL: Duration = Duration::from_millis(33);

/// What the pump sends to the window.
enum Update {
    /// A frame to display.
    Frame(Arc<Frame>),
    /// Why there will be no more frames.
    Problem(String),
}

/// Show the desktop until the operator closes the window.
///
/// Returns once the window is gone. The capture session in `backends` is left
/// alone: the window is a view of the session, not its owner, so closing it must
/// not end the session or stop the server.
pub fn run(backends: Arc<Backends>) -> Result<(), Error> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("computer-use — what the model sees")
            .with_inner_size([900.0, 600.0]),
        ..Default::default()
    };

    eframe::run_native(
        "computer-use preview",
        options,
        Box::new(move |cc| Ok(Box::new(PreviewApp::new(cc, backends)))),
    )
    .map_err(|error| Error::PreviewWindow(error.to_string()))
}

/// The window itself.
struct PreviewApp {
    /// Frames and problems from the pump.
    updates: Receiver<Update>,
    /// The one texture, reused for the window's whole life.
    texture: Option<egui::TextureHandle>,
    /// The size the texture currently holds, so a change reallocates it.
    texture_size: [usize; 2],
    /// Set when the pump could not continue; shown in place of an image.
    problem: Option<String>,
}

impl PreviewApp {
    fn new(cc: &eframe::CreationContext<'_>, backends: Arc<Backends>) -> Self {
        let (sender, updates) = std::sync::mpsc::channel();
        let context = cc.egui_ctx.clone();

        // The pump gets its own thread and its own runtime rather than sharing
        // the server's: it polls continuously, and a server task waiting behind
        // frame delivery would make every tool call as slow as the preview.
        std::thread::Builder::new()
            .name("preview-pump".to_string())
            .spawn(move || pump(backends, sender, context))
            .expect("the preview pump thread could not be started");

        Self {
            updates,
            texture: None,
            texture_size: [0, 0],
            problem: None,
        }
    }

    /// Upload a frame, reusing the texture when its size has not changed.
    ///
    /// Allocating a texture per frame is the obvious way to leak GPU memory here,
    /// so re-uploading into the existing one is the point rather than a
    /// refinement.
    fn upload(&mut self, context: &egui::Context, frame: &Frame) {
        let size = [frame.width as usize, frame.height as usize];
        let image = egui::ColorImage::from_rgb(size, &frame.rgb);

        if self.texture.is_some() && self.texture_size == size {
            if let Some(texture) = &mut self.texture {
                texture.set(image, egui::TextureOptions::LINEAR);
            }
            return;
        }

        self.texture =
            Some(context.load_texture("computer-use-frame", image, egui::TextureOptions::LINEAR));
        self.texture_size = size;
    }
}

impl eframe::App for PreviewApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if let Some(frame) = take_newest(&self.updates, &mut self.problem) {
            self.problem = None;
            let context = ui.ctx().clone();
            self.upload(&context, &frame);
        }

        match (&self.problem, &self.texture) {
            (Some(problem), _) => {
                ui.label("The preview stopped. The server is unaffected and still running.");
                ui.label(problem);
            }
            (None, Some(texture)) => {
                ui.add(
                    egui::Image::new(texture)
                        .maintain_aspect_ratio(true)
                        .shrink_to_fit(),
                );
            }
            (None, None) => {
                ui.label("Waiting for the first frame…");
                ui.label(
                    "If a permission prompt is on screen, it needs an answer before \
                     anything can be captured.",
                );
            }
        }
    }
}

/// Take the newest frame queued for display, or the first problem reported.
///
/// Everything else queued is dropped. The point is to show what the model sees
/// *now*, so working through a backlog would mean displaying stale frames late,
/// which is the opposite of what a preview is for.
///
/// A free function over the receiver rather than a method, so it can be tested
/// without constructing a window.
fn take_newest(updates: &Receiver<Update>, problem: &mut Option<String>) -> Option<Arc<Frame>> {
    let mut newest = None;
    loop {
        match updates.try_recv() {
            Ok(Update::Frame(frame)) => newest = Some(frame),
            Ok(Update::Problem(reported)) => {
                *problem = Some(reported);
                break;
            }
            Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
        }
    }
    newest
}

/// Poll the capture backend and forward frames to the window.
///
/// Stops at the first failure rather than retrying. A failure here is usually a
/// denied consent prompt, and retrying on a timer would ask again and again —
/// worse than showing the reason once and leaving the operator to restart.
fn pump(backends: Arc<Backends>, updates: Sender<Update>, context: egui::Context) {
    tracing::debug!("the preview pump started");

    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            return report(
                &updates,
                &context,
                format!("could not start the preview runtime: {error}"),
            );
        }
    };

    let Some(capture) = backends.capture.as_ref() else {
        return report(
            &updates,
            &context,
            "no capture backend is available, so there is nothing to preview".to_string(),
        );
    };

    let mut receiving = false;

    runtime.block_on(async {
        loop {
            match capture.capture_frame().await {
                Ok(frame) => {
                    if !receiving {
                        receiving = true;
                        tracing::debug!(
                            "the preview is receiving frames ({}x{})",
                            frame.width,
                            frame.height
                        );
                    }
                    // A failed send means the window is gone, which is the signal
                    // to stop.
                    if updates.send(Update::Frame(frame)).is_err() {
                        return;
                    }
                    context.request_repaint();
                }
                Err(error) => return report(&updates, &context, error.to_string()),
            }

            tokio::time::sleep(POLL_INTERVAL).await;
        }
    });
}

/// Tell the window why nothing more is coming, and wake it so it shows it.
///
/// Logged as well as displayed: a failure that only appears in the window cannot
/// be diagnosed when the window is the very thing being tested, which is exactly
/// what happened on the first run of this code.
fn report(updates: &Sender<Update>, context: &egui::Context, problem: String) {
    tracing::error!("the preview stopped: {problem}");
    let _ = updates.send(Update::Problem(problem));
    context.request_repaint();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(width: u32) -> Arc<Frame> {
        Arc::new(Frame {
            width,
            height: 1,
            rgb: vec![0; Frame::expected_len(width, 1)],
        })
    }

    #[test]
    fn the_newest_frame_wins_and_the_older_ones_are_dropped() {
        let (sender, updates) = std::sync::mpsc::channel();
        sender.send(Update::Frame(frame(1))).expect("send");
        sender.send(Update::Frame(frame(2))).expect("send");
        sender.send(Update::Frame(frame(3))).expect("send");

        let mut problem = None;
        let newest = take_newest(&updates, &mut problem).expect("a frame");

        assert_eq!(newest.width, 3, "the most recent frame should win");
        assert!(problem.is_none());
    }

    #[test]
    fn nothing_queued_produces_no_update() {
        let (_sender, updates) = std::sync::mpsc::channel::<Update>();

        let mut problem = None;
        assert!(take_newest(&updates, &mut problem).is_none());
        assert!(problem.is_none());
    }

    #[test]
    fn a_problem_is_reported() {
        let (sender, updates) = std::sync::mpsc::channel();
        sender
            .send(Update::Problem("the portal refused".to_string()))
            .expect("send");

        let mut problem = None;
        assert!(take_newest(&updates, &mut problem).is_none());
        assert_eq!(problem.as_deref(), Some("the portal refused"));
    }

    #[test]
    fn a_problem_arriving_after_frames_is_still_reported() {
        let (sender, updates) = std::sync::mpsc::channel();
        sender.send(Update::Frame(frame(1))).expect("send");
        sender
            .send(Update::Problem("the stream stopped".to_string()))
            .expect("send");

        let mut problem = None;
        let newest = take_newest(&updates, &mut problem).expect("a frame");

        assert_eq!(newest.width, 1);
        assert_eq!(problem.as_deref(), Some("the stream stopped"));
    }

    #[test]
    fn a_closed_channel_is_the_window_ending_not_a_problem() {
        let (sender, updates) = std::sync::mpsc::channel::<Update>();
        drop(sender);

        let mut problem = None;
        assert!(take_newest(&updates, &mut problem).is_none());
        assert!(
            problem.is_none(),
            "the window closing is not something to report to the operator"
        );
    }
}
