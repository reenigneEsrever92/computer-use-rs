//! The PipeWire frame source.
//!
//! The portal hands us a PipeWire remote file descriptor and a node id; this
//! subscribes to that node and turns the buffers it delivers into RGB frames.
//!
//! PipeWire's objects are neither `Send` nor `Sync`, so a dedicated thread
//! creates, owns, and drops every one of them. The rest of the program talks to
//! the thread over a `watch` channel, and asks it to stop over a PipeWire
//! channel — the safe API cannot quit a loop from another thread, so the quit
//! has to be posted onto the loop's own dispatcher.

use std::io::Cursor;
use std::os::fd::OwnedFd;
use std::sync::Arc;
use std::sync::mpsc::{SyncSender, sync_channel};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use pipewire as pw;
use pw::properties::properties;
use pw::spa;
use pw::spa::pod::{Pod, object, property};
use pw::stream::{StreamFlags, StreamListener, StreamRc};
use tokio::sync::watch;

use crate::backend::capture::Frame;
use crate::backend::convert::{PixelLayout, to_rgb};
use crate::error::Error;

/// How long to wait for a frame before deciding capture is broken.
pub const DEFAULT_FRAME_TIMEOUT: Duration = Duration::from_secs(20);

/// What the capture thread is currently producing.
#[derive(Debug, Clone, Default)]
pub struct SourceState {
    /// The most recent frame.
    pub frame: Option<Arc<Frame>>,
    /// The problem that stopped frames being produced, if one is known.
    pub error: Option<String>,
    /// The negotiated frame size, once the format is known.
    pub size: Option<(u32, u32)>,
}

/// Commands posted onto the PipeWire loop.
enum Command {
    /// Stop the loop, which unwinds the thread.
    Quit,
}

/// State the stream callbacks share.
struct UserData {
    /// The format the compositor settled on; filled in by `param_changed`.
    format: spa::param::video::VideoInfoRaw,
    /// Where converted frames are published.
    publisher: watch::Sender<SourceState>,
    /// Whether the negotiated stream has been reported once.
    reported: bool,
    /// The loop to stop when the stream fails.
    ///
    /// A stream in an error state will never deliver another frame, so the thread
    /// should not sit idle waiting for one.
    mainloop: pw::main_loop::MainLoopRc,
}

impl UserData {
    /// Record a failure, keeping the first one.
    ///
    /// The first failure is the specific one — "the compositor delivered
    /// DmaBuf buffers" — while later ones tend to be its consequences.
    fn fail(&mut self, message: impl Into<String>) {
        let mut state = SourceState::clone(&self.publisher.borrow());
        if state.error.is_some() {
            return;
        }
        state.error = Some(message.into());
        let _ = self.publisher.send(state);
    }
}

/// A running PipeWire stream delivering frames from the compositor.
pub struct FrameSource {
    state: watch::Receiver<SourceState>,
    commands: pw::channel::Sender<Command>,
    handle: Option<JoinHandle<()>>,
}

impl FrameSource {
    /// Connect to the node the portal selected and start producing frames.
    ///
    /// Returns once the stream is connected, so that a setup failure is reported
    /// here rather than surfacing later as a mysterious absence of frames.
    pub fn start(fd: OwnedFd, node_id: u32) -> Result<Self, Error> {
        let (state_tx, state_rx) = watch::channel(SourceState::default());
        let (command_tx, command_rx) = pw::channel::channel::<Command>();
        let (ready_tx, ready_rx) = sync_channel::<Result<(), Error>>(1);

        let handle = thread::Builder::new()
            .name("pipewire-capture".to_string())
            .spawn(move || {
                if let Err(error) = run(fd, node_id, command_rx, ready_tx, state_tx) {
                    tracing::error!("the PipeWire capture thread stopped: {error}");
                }
            })
            .map_err(Error::Runtime)?;

        match ready_rx.recv() {
            Ok(Ok(())) => Ok(Self {
                state: state_rx,
                commands: command_tx,
                handle: Some(handle),
            }),
            Ok(Err(error)) => {
                let _ = handle.join();
                Err(error)
            }
            Err(_) => {
                let _ = handle.join();
                Err(Error::CaptureThreadStopped)
            }
        }
    }

    /// The most recent state, for diagnostics.
    pub fn state(&self) -> SourceState {
        SourceState::clone(&self.state.borrow())
    }

    /// Whether this source can still deliver a frame.
    ///
    /// False once the capture thread has stopped, or the stream has reported an
    /// error. This is how a caller tells a live session from one that was torn
    /// down underneath it: without it, the last good frame would be served
    /// indefinitely and would look like the current desktop.
    pub fn is_alive(&self) -> bool {
        let thread_running = self
            .handle
            .as_ref()
            .is_some_and(|handle| !handle.is_finished());
        thread_running && self.state.borrow().error.is_none()
    }

    /// The stream's negotiated size, waiting for it if it is not known yet.
    pub async fn size(&mut self) -> Result<(u32, u32), Error> {
        self.wait_for_size(DEFAULT_FRAME_TIMEOUT).await
    }

    /// Wait until the stream has settled on a size.
    ///
    /// The size is known as soon as the format is negotiated, which is before
    /// the first frame, so this is what lets the scale factor be computed without
    /// waiting for content.
    pub async fn wait_for_size(&mut self, timeout: Duration) -> Result<(u32, u32), Error> {
        let settled = tokio::time::timeout(timeout, async {
            loop {
                {
                    let state = self.state.borrow();
                    if let Some(error) = state.error.clone() {
                        return Err(Error::Capture(error));
                    }
                    if let Some(size) = state.size {
                        return Ok(size);
                    }
                }

                if self.state.changed().await.is_err() {
                    return Err(Error::Capture(
                        "the capture thread stopped before the stream negotiated a size"
                            .to_string(),
                    ));
                }
            }
        })
        .await;

        match settled {
            Ok(result) => result,
            Err(_) => Err(Error::FrameTimeout {
                seconds: timeout.as_secs(),
            }),
        }
    }

    /// Wait for a frame, giving up after [`DEFAULT_FRAME_TIMEOUT`].
    pub async fn capture(&mut self) -> Result<Arc<Frame>, Error> {
        self.capture_within(DEFAULT_FRAME_TIMEOUT).await
    }

    /// Wait for a frame, giving up after `timeout`.
    pub async fn capture_within(&mut self, timeout: Duration) -> Result<Arc<Frame>, Error> {
        match tokio::time::timeout(timeout, self.wait_for_frame()).await {
            Ok(result) => result,
            Err(_) => Err(Error::FrameTimeout {
                seconds: timeout.as_secs(),
            }),
        }
    }

    /// Stop the stream and wait for the capture thread to unwind.
    pub fn stop(&mut self) {
        let _ = self.commands.send(Command::Quit);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }

    /// The most recent frame, or the reason there is none.
    async fn wait_for_frame(&mut self) -> Result<Arc<Frame>, Error> {
        loop {
            {
                let state = self.state.borrow();
                if let Some(error) = state.error.clone() {
                    return Err(Error::Capture(error));
                }
                if let Some(frame) = state.frame.clone() {
                    return Ok(frame);
                }
            }

            // The sender lives in the capture thread, so a closed channel means
            // the thread is gone and no frame is ever coming.
            if self.state.changed().await.is_err() {
                return Err(Error::Capture(
                    "the capture thread stopped before producing a frame".to_string(),
                ));
            }
        }
    }
}

impl Drop for FrameSource {
    fn drop(&mut self) {
        self.stop();
    }
}

/// The capture thread: set everything up, then run the loop until asked to stop.
fn run(
    fd: OwnedFd,
    node_id: u32,
    commands: pw::channel::Receiver<Command>,
    ready: SyncSender<Result<(), Error>>,
    state_tx: watch::Sender<SourceState>,
) -> Result<(), Error> {
    pw::init();

    let mainloop = match pw::main_loop::MainLoopRc::new(None) {
        Ok(mainloop) => mainloop,
        Err(error) => {
            let _ = ready.send(Err(error.into()));
            return Ok(());
        }
    };

    // Stop requests are posted onto the loop, because `quit` takes `&self` on a
    // type that cannot be sent between threads.
    let quit = mainloop.clone();
    let _commands = commands.attach(mainloop.loop_(), move |_| quit.quit());

    // Held for the rest of the function: the stream, its callbacks, and the
    // parameters it was connected with all have to outlive the loop. They are
    // dropped before the loop because they are declared after it.
    let _connection = match build_stream(&mainloop, fd, node_id, state_tx) {
        Ok(connection) => {
            let _ = ready.send(Ok(()));
            connection
        }
        Err(error) => {
            let _ = ready.send(Err(error));
            return Ok(());
        }
    };

    mainloop.run();
    Ok(())
}

/// Everything the connection needs to keep alive for the stream's lifetime.
///
/// The serialized pods are held because libpipewire may retain pointers into
/// them after `connect` returns.
#[allow(dead_code)]
struct ConnectedStream {
    /// The registered callbacks; dropping this unhooks them.
    listener: StreamListener<UserData>,
    /// The stream itself.
    stream: StreamRc,
    /// The serialized parameters the stream was connected with.
    params: Vec<Vec<u8>>,
}

/// Serialize one pod into the bytes a `connect` call wants.
fn serialize_pod(value: &spa::pod::Value) -> Result<Vec<u8>, Error> {
    spa::pod::serialize::PodSerializer::serialize(Cursor::new(Vec::new()), value)
        .map(|(cursor, _length)| cursor.into_inner())
        .map_err(|error| Error::Capture(format!("could not serialize stream parameters: {error}")))
}

/// Ask the producer for shared-memory buffers.
///
/// `SPA_PARAM_BUFFERS_dataType` is a mask of the memory types the consumer can
/// use. Only shared memory is acceptable here: a DMA-BUF plane is reachable
/// solely as a file descriptor, and this crate has no way to import one.
///
/// Saying so explicitly is stronger than the indirect signal sent by leaving the
/// video modifier out of the format negotiation, which is all that keeps GNOME on
/// shared memory today. Whether a given producer honours it is a server-side
/// decision, so the frame source still checks what it actually receives.
///
/// The `property!` macro has no plain-integer shape, so the object is built by
/// hand rather than with the macro.
fn shared_memory_buffers_param() -> spa::pod::Value {
    let acceptable =
        (1u32 << pw::spa::sys::SPA_DATA_MemFd) | (1u32 << pw::spa::sys::SPA_DATA_MemPtr);

    spa::pod::Value::Object(spa::pod::Object {
        type_: spa::utils::SpaTypes::ObjectParamBuffers.as_raw(),
        id: spa::param::ParamType::Buffers.as_raw(),
        properties: vec![spa::pod::Property {
            key: pw::spa::sys::SPA_PARAM_BUFFERS_dataType,
            flags: spa::pod::PropertyFlags::empty(),
            value: spa::pod::Value::Int(acceptable as i32),
        }],
    })
}

/// Create and connect the stream, and register the callbacks.
fn build_stream(
    mainloop: &pw::main_loop::MainLoopRc,
    fd: OwnedFd,
    node_id: u32,
    publisher: watch::Sender<SourceState>,
) -> Result<ConnectedStream, Error> {
    let context = pw::context::ContextRc::new(mainloop, None)?;
    // Consumes the fd: libpipewire closes the socket when the core disconnects.
    let core = context.connect_fd_rc(fd, None)?;

    let stream = StreamRc::new(
        core,
        "computer-use-capture",
        properties! {
            *pw::keys::MEDIA_TYPE => "Video",
            *pw::keys::MEDIA_CATEGORY => "Capture",
            *pw::keys::MEDIA_ROLE => "Screen",
        },
    )?;

    let user_data = UserData {
        format: spa::param::video::VideoInfoRaw::default(),
        publisher,
        reported: false,
        mainloop: mainloop.clone(),
    };

    let listener = stream
        .add_local_listener_with_user_data(user_data)
        .state_changed(|_, user_data, _old, new| {
            // Reaching this state means no further frame will ever arrive, so
            // record why and stop, rather than leaving the capture thread idle and
            // the last good frame looking current.
            if let pw::stream::StreamState::Error(message) = new {
                user_data.fail(format!("the PipeWire stream failed: {message}"));
                user_data.mainloop.quit();
            }
        })
        .param_changed(|_, user_data, id, param| {
            let Some(param) = param else {
                return;
            };
            if id != spa::param::ParamType::Format.as_raw() {
                return;
            }

            let Ok((media_type, media_subtype)) = spa::param::format_utils::parse_format(param)
            else {
                return;
            };
            if media_type != spa::param::format::MediaType::Video
                || media_subtype != spa::param::format::MediaSubtype::Raw
            {
                return;
            }

            if let Err(error) = user_data.format.parse(param) {
                user_data.fail(format!(
                    "could not read the negotiated video format: {error}"
                ));
                return;
            }

            let size = user_data.format.size();
            let mut state = SourceState::clone(&user_data.publisher.borrow());
            state.size = Some((size.width, size.height));
            let _ = user_data.publisher.send(state);
        })
        .process(|stream, user_data| {
            let Some(mut buffer) = stream.dequeue_buffer() else {
                return;
            };
            let Some(plane) = buffer.datas_mut().first_mut() else {
                return;
            };

            let size = user_data.format.size();
            let (width, height) = (size.width, size.height);
            if width == 0 || height == 0 {
                return;
            }

            // Geometry first: reading it needs a shared borrow, and the pixels
            // need a mutable one.
            let (offset, length, stride) = {
                let chunk = plane.chunk();
                (
                    chunk.offset() as usize,
                    chunk.size() as usize,
                    chunk.stride(),
                )
            };
            let data_type = plane.type_();

            // This is the DMA-BUF cliff. `MAP_BUFFERS` mmaps shared memory and
            // `MemPtr` is already a pointer, but a DMA-BUF plane is only reachable
            // through a file descriptor, which this crate cannot import.
            let Some(pixels) = plane.data() else {
                user_data.fail(format!(
                    "the compositor delivered {data_type:?} buffers, which this build cannot \
                     read: they are reachable only as a file descriptor. This portal backend \
                     did not fall back to shared memory"
                ));
                return;
            };

            let Some(layout) = PixelLayout::from_video_format(user_data.format.format()) else {
                user_data.fail(format!(
                    "the compositor settled on {:?}, which this build has no conversion for",
                    user_data.format.format()
                ));
                return;
            };

            // A negative stride means the rows are stored bottom-up, which we do
            // not handle; treat it as tightly packed rather than reading garbage.
            let stride = if stride > 0 {
                stride as usize
            } else {
                width as usize * PixelLayout::BYTES_PER_PIXEL
            };

            if !user_data.reported {
                user_data.reported = true;
                tracing::info!(
                    "stream negotiated: {width}x{height}, {layout:?} layout, in {data_type:?} \
                     buffers, stride {stride}"
                );
            }

            let end = offset.saturating_add(length).min(pixels.len());
            if offset >= end {
                return;
            }

            let Some(rgb) = to_rgb(layout, &pixels[offset..end], width, height, stride) else {
                user_data.fail(format!(
                    "the compositor's buffer for {width}x{height} was shorter than its own \
                     stride implies"
                ));
                return;
            };

            let _ = user_data.publisher.send(SourceState {
                frame: Some(Arc::new(Frame { width, height, rgb })),
                error: None,
                size: Some((width, height)),
            });
        })
        .register()?;

    let format_object = object!(
        spa::utils::SpaTypes::ObjectParamFormat,
        spa::param::ParamType::EnumFormat,
        property!(
            spa::param::format::FormatProperties::MediaType,
            Id,
            spa::param::format::MediaType::Video
        ),
        property!(
            spa::param::format::FormatProperties::MediaSubtype,
            Id,
            spa::param::format::MediaSubtype::Raw
        ),
        // The layouts we can convert. There is deliberately no VideoModifier
        // property: the modifier key is documented as DMA-BUF-only, so offering
        // it would invite exactly the buffer type we cannot read.
        property!(
            spa::param::format::FormatProperties::VideoFormat,
            Choice,
            Enum,
            Id,
            spa::param::video::VideoFormat::BGRx,
            spa::param::video::VideoFormat::BGRx,
            spa::param::video::VideoFormat::RGBx,
            spa::param::video::VideoFormat::BGRA,
            spa::param::video::VideoFormat::RGBA
        ),
        property!(
            spa::param::format::FormatProperties::VideoSize,
            Choice,
            Range,
            Rectangle,
            spa::utils::Rectangle {
                width: 1920,
                height: 1080
            },
            spa::utils::Rectangle {
                width: 1,
                height: 1
            },
            spa::utils::Rectangle {
                width: 16384,
                height: 16384
            }
        ),
    );

    // Two parameters: the formats we accept, and the memory we can read them out
    // of. The second is what keeps the negotiation on shared memory.
    let params = vec![
        serialize_pod(&spa::pod::Value::Object(format_object))?,
        serialize_pod(&shared_memory_buffers_param())?,
    ];

    {
        let mut pods = [
            Pod::from_bytes(&params[0]).ok_or_else(|| {
                Error::Capture("the serialized video format parameters were not valid".into())
            })?,
            Pod::from_bytes(&params[1]).ok_or_else(|| {
                Error::Capture("the serialized buffer parameters were not valid".into())
            })?,
        ];

        stream.connect(
            spa::utils::Direction::Input,
            Some(node_id),
            StreamFlags::AUTOCONNECT | StreamFlags::MAP_BUFFERS,
            &mut pods,
        )?;
    }

    Ok(ConnectedStream {
        listener,
        stream,
        params,
    })
}
