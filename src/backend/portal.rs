//! The portal backend: capture and input over one shared session.
//!
//! Capture is negotiated through the XDG Desktop Portal rather than through a
//! compositor-specific protocol, which is what makes one implementation work on
//! GNOME, KDE, and wlroots alike. Input goes through the same portal, on the same
//! session, deliberately: one session means one consent prompt for both, which is
//! what the requirements ask for. A second session would ask the user twice.
//!
//! The session is created through `RemoteDesktop` because only that interface's
//! `Start` returns the devices *and* the streams. A portal without it — wlroots,
//! where `xdg-desktop-portal-wlr` implements screen casting only — falls back to
//! a `ScreenCast`-owned session, so capture keeps working and input reports that
//! it cannot.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use ashpd::desktop::remote_desktop::{
    Axis as PortalAxis, DeviceType, KeyState as PortalKeyState, RemoteDesktop,
};
use ashpd::desktop::screencast::{CursorMode, Screencast, SourceType, Stream};
use ashpd::desktop::{PersistMode, Session};
use tokio::sync::Mutex;

use crate::backend::capture::{BoxFuture, CaptureBackend, CaptureInfo, Frame};
use crate::backend::frame_source::FrameSource;
use crate::backend::input::{Axis, Button, InputBackend, KeyState};
use crate::backend::keys::Keysym;
use crate::error::Error;

/// How the screencast interface is identified in errors from the portal.
const SCREENCAST: &str = "org.freedesktop.portal.ScreenCast";
/// How the remote desktop interface is identified in errors from the portal.
const REMOTE_DESKTOP: &str = crate::error::REMOTE_DESKTOP_INTERFACE;

/// How long a single capture will wait for the compositor to deliver a frame.
const FRAME_TIMEOUT: Duration = Duration::from_secs(20);

/// The evdev button codes the portal expects.
///
/// Not an enum in the binding: the portal takes raw evdev codes, so the names
/// stop here.
mod button_code {
    /// The primary button.
    pub const LEFT: i32 = 0x110;
    /// The middle button.
    pub const MIDDLE: i32 = 0x112;
    /// The secondary button.
    pub const RIGHT: i32 = 0x111;
}

/// A portal session serving both capture and input.
pub struct PortalSession {
    /// Established on first use: the portal session is only worth creating when a
    /// tool actually needs the desktop, and creating it prompts for consent.
    established: Mutex<Option<Established>>,
}

/// The two shapes a session can take, depending on what the portal provides.
enum SessionHandle {
    /// A session with input devices, created through `RemoteDesktop`.
    WithInput(Session<'static, RemoteDesktop<'static>>),
    /// A capture-only session, for a portal without `RemoteDesktop`.
    ///
    /// The handle is never read: it is held because dropping it would tear down
    /// the portal session the frames are still arriving from.
    #[allow(dead_code)]
    CaptureOnly(Session<'static, Screencast<'static>>),
}

/// A live portal session and the frames coming from it.
struct Established {
    session: SessionHandle,
    /// Present only when the portal granted input devices.
    remote_desktop: Option<RemoteDesktop<'static>>,
    /// What the platform reported about the desktop.
    info: CaptureInfo,
    /// The PipeWire node the input events are addressed to.
    node_id: u32,
    /// The frames arriving from the compositor.
    frames: FrameSource,
}

impl Default for PortalSession {
    fn default() -> Self {
        Self::new()
    }
}

impl PortalSession {
    /// Create a backend. Nothing is negotiated until it is first used.
    pub fn new() -> Self {
        Self {
            established: Mutex::new(None),
        }
    }

    /// Get the established session, opening one if this is the first use.
    ///
    /// The guard is returned rather than the session so callers can borrow it
    /// across an await — waiting for a frame needs the session to stay put.
    async fn ensure(&self) -> Result<tokio::sync::MutexGuard<'_, Option<Established>>, Error> {
        let mut guard = self.established.lock().await;
        if guard.is_none() {
            *guard = Some(Established::open().await?);
        }
        Ok(guard)
    }

    /// One capture against whatever session currently exists.
    async fn attempt_capture(&self) -> Result<Arc<Frame>, Error> {
        let mut guard = self.ensure().await?;
        let established = guard.as_mut().expect("just established");
        // Handed over as the `Arc` the frame source already holds, so a caller
        // that only wants to look at the frame copies nothing.
        established.frames.capture_within(FRAME_TIMEOUT).await
    }
}

impl CaptureBackend for PortalSession {
    fn name(&self) -> &'static str {
        "portal+pipewire"
    }

    fn connect(&self) -> BoxFuture<'_, Result<CaptureInfo, Error>> {
        Box::pin(async move {
            let guard = self.ensure().await?;
            let established = guard.as_ref().expect("just established");
            Ok(established.info.clone())
        })
    }

    fn capture_frame(&self) -> BoxFuture<'_, Result<Arc<Frame>, Error>> {
        Box::pin(async move {
            match self.attempt_capture().await {
                Ok(frame) => Ok(frame),
                Err(error) => {
                    // A session can be torn down underneath us — the compositor
                    // restarts, the captured source is removed. If that is what
                    // happened, the session is discarded and one fresh session is
                    // tried, so a restart costs a re-negotiation rather than
                    // failing until the process restarts.
                    let stale = {
                        let guard = self.established.lock().await;
                        guard
                            .as_ref()
                            .is_some_and(|established| !established.frames.is_alive())
                    };
                    if !stale {
                        return Err(error);
                    }

                    tracing::warn!(
                        "capture failed and the session is no longer alive ({error}); \
                         establishing a new session"
                    );
                    *self.established.lock().await = None;
                    self.attempt_capture().await
                }
            }
        })
    }
}

impl InputBackend for PortalSession {
    fn name(&self) -> &'static str {
        "portal+pipewire"
    }

    fn connect(&self) -> BoxFuture<'_, Result<(), Error>> {
        Box::pin(async move {
            // Establishing is the same negotiation capture performs, so this is
            // where consent is obtained if capture has not been used yet.
            let _guard = self.ensure().await?;
            Ok(())
        })
    }

    fn pointer_move(&self, x: f64, y: f64) -> BoxFuture<'_, Result<(), Error>> {
        Box::pin(async move {
            let mut guard = self.ensure().await?;
            let established = guard.as_mut().expect("just established");

            // The tools speak logical desktop coordinates; the portal's absolute
            // motion is specified in the stream's pixels. Applying the scale
            // factor here keeps that conversion in the one place it is needed.
            let (stream_x, stream_y) = established.info.logical_to_stream((x, y));

            {
                let (remote_desktop, session, node, _) = established.input_parts()?;
                remote_desktop
                    .notify_pointer_motion_absolute(session, node, stream_x, stream_y)
                    .await
                    .map_err(|error| Error::Input {
                        detail: format!("moving the pointer to ({x}, {y}): {error}"),
                    })?;
            }

            // The portal reports no pointer position, so the most this session can
            // know is where it last sent the pointer. Recording it here is what
            // makes `cursor_position` and the metadata's pointer field mean
            // something instead of being permanently null.
            established.info.pointer = Some((x, y));
            Ok(())
        })
    }

    fn pointer_button(&self, button: Button, state: KeyState) -> BoxFuture<'_, Result<(), Error>> {
        Box::pin(async move {
            let code = button_code_for(button);
            let state = to_portal_state(state);

            let mut guard = self.ensure().await?;
            let established = guard.as_mut().expect("just established");
            let (remote_desktop, session, _, _) = established.input_parts()?;

            remote_desktop
                .notify_pointer_button(session, code, state)
                .await
                .map_err(input_error)
        })
    }

    fn pointer_scroll(&self, axis: Axis, amount: f64) -> BoxFuture<'_, Result<(), Error>> {
        Box::pin(async move {
            // Whole wheel steps: the discrete axis is what a mouse wheel actually
            // produces, and it keeps "scroll down three" meaning the same thing
            // from one compositor to the next.
            let steps = amount.round() as i32;
            if steps == 0 {
                return Ok(());
            }

            let axis = match axis {
                Axis::Vertical => PortalAxis::Vertical,
                Axis::Horizontal => PortalAxis::Horizontal,
            };

            let mut guard = self.ensure().await?;
            let established = guard.as_mut().expect("just established");
            let (remote_desktop, session, _, _) = established.input_parts()?;

            remote_desktop
                .notify_pointer_axis_discrete(session, axis, steps)
                .await
                .map_err(input_error)
        })
    }

    fn key(&self, keysym: Keysym, state: KeyState) -> BoxFuture<'_, Result<(), Error>> {
        Box::pin(async move {
            let state = to_portal_state(state);

            let mut guard = self.ensure().await?;
            let established = guard.as_mut().expect("just established");
            let (remote_desktop, session, _, _) = established.input_parts()?;

            remote_desktop
                .notify_keyboard_keysym(session, keysym as i32, state)
                .await
                .map_err(input_error)
        })
    }
}

/// The evdev code for a named button.
fn button_code_for(button: Button) -> i32 {
    match button {
        Button::Left => button_code::LEFT,
        Button::Middle => button_code::MIDDLE,
        Button::Right => button_code::RIGHT,
    }
}

/// The portal's spelling of the shared press/release state.
fn to_portal_state(state: KeyState) -> PortalKeyState {
    match state {
        KeyState::Pressed => PortalKeyState::Pressed,
        KeyState::Released => PortalKeyState::Released,
    }
}

/// Map a failure to send an event onto an actionable error.
fn input_error(error: ashpd::Error) -> Error {
    Error::Input {
        detail: error.to_string(),
    }
}

impl Established {
    /// The pieces needed to inject input, or the reason there are none.
    #[allow(clippy::type_complexity)]
    fn input_parts(
        &self,
    ) -> Result<
        (
            &RemoteDesktop<'static>,
            &Session<'static, RemoteDesktop<'static>>,
            u32,
            &CaptureInfo,
        ),
        Error,
    > {
        let remote_desktop =
            self.remote_desktop
                .as_ref()
                .ok_or_else(|| Error::PortalInterfaceUnavailable {
                    interface: REMOTE_DESKTOP,
                    detail: "this session granted no input devices, so there is nothing to send \
                         events to. A wlroots session reaches this state, because \
                         xdg-desktop-portal-wlr implements screen casting but not remote \
                         desktop"
                        .to_string(),
                })?;

        let SessionHandle::WithInput(session) = &self.session else {
            return Err(Error::PortalInterfaceUnavailable {
                interface: REMOTE_DESKTOP,
                detail: "this session was established for capture only".to_string(),
            });
        };

        Ok((remote_desktop, session, self.node_id, &self.info))
    }

    /// Negotiate a session with the portal and start reading frames from it.
    async fn open() -> Result<Self, Error> {
        // A restore token is what keeps the consent prompt from appearing on
        // every run. If the user revoked it, the portal rejects it, and the retry
        // without one is what re-prompts.
        match load_restore_token() {
            Some(token) => match Self::negotiate(Some(&token)).await {
                Ok(established) => Ok(established),
                Err(error) => {
                    tracing::warn!(
                        "the stored portal consent token was not accepted ({error}); \
                         asking again"
                    );
                    Self::negotiate(None).await
                }
            },
            None => Self::negotiate(None).await,
        }
    }

    /// Try for a session with input, falling back to capture alone.
    ///
    /// The fallback exists because the session is created through
    /// `RemoteDesktop`: without it, a portal that only implements screen casting
    /// would fail the whole negotiation, and capture would stop working on
    /// wlroots as well as input.
    async fn negotiate(restore_token: Option<&str>) -> Result<Self, Error> {
        match Self::negotiate_with_input(restore_token).await {
            Ok(established) => Ok(established),
            Err(error) if error.is_input_unsupported() => {
                tracing::warn!(
                    "this portal does not provide RemoteDesktop ({error}); falling back to \
                     capture only, so the input tools will not work"
                );
                Self::negotiate_capture_only(restore_token).await
            }
            Err(error) => Err(error),
        }
    }

    /// A session with capture and input, from one consent prompt.
    async fn negotiate_with_input(restore_token: Option<&str>) -> Result<Self, Error> {
        let remote_desktop: RemoteDesktop<'static> = RemoteDesktop::new()
            .await
            .map_err(portal_error(REMOTE_DESKTOP))?;
        let screencast: Screencast<'static> =
            Screencast::new().await.map_err(portal_error(SCREENCAST))?;

        // The session belongs to RemoteDesktop so that its `start` returns both
        // the devices and the streams.
        let session = remote_desktop
            .create_session()
            .await
            .map_err(portal_error(REMOTE_DESKTOP))?;

        remote_desktop
            .select_devices(
                &session,
                DeviceType::Keyboard | DeviceType::Pointer,
                restore_token,
                PersistMode::Application,
            )
            .await
            .map_err(portal_error(REMOTE_DESKTOP))?
            .response()
            .map_err(portal_error(REMOTE_DESKTOP))?;

        screencast
            .select_sources(
                &session,
                // Embedded rather than metadata: with metadata the cursor is
                // *not* in the pixels, and this build does not yet read the
                // cursor buffer metadata that would tell it where the pointer is.
                CursorMode::Embedded,
                SourceType::Monitor.into(),
                false,
                // Neither a token nor persistence here: this session is a
                // remote-desktop session, and the portal rejects persistence on
                // those outright ("Remote desktop sessions cannot persist").
                // Consent for the combined session is persisted through
                // `select_devices` above.
                None,
                PersistMode::DoNot,
            )
            .await
            .map_err(portal_error(SCREENCAST))?
            .response()
            .map_err(portal_error(SCREENCAST))?;

        let selected = remote_desktop
            .start(&session, None)
            .await
            .map_err(portal_error(REMOTE_DESKTOP))?
            .response()
            .map_err(portal_error(REMOTE_DESKTOP))?;

        if let Some(token) = selected.restore_token() {
            store_restore_token(token);
        }

        let stream = first_stream(selected.streams())?;
        let devices = selected.devices();
        let input_available =
            devices.contains(DeviceType::Keyboard) && devices.contains(DeviceType::Pointer);

        if !input_available {
            tracing::warn!(
                "this portal session granted no input devices; the input tools will not work"
            );
        }

        let fd = screencast
            .open_pipe_wire_remote(&session)
            .await
            .map_err(portal_error(SCREENCAST))?;

        let node_id = stream.pipe_wire_node_id();
        let mut frames = FrameSource::start(fd, node_id)?;
        let stream_size = frames.size().await?;
        let info = describe(&stream, stream_size, input_available);

        Ok(Self {
            session: SessionHandle::WithInput(session),
            remote_desktop: Some(remote_desktop),
            info,
            node_id,
            frames,
        })
    }

    /// A capture-only session, for a portal without `RemoteDesktop`.
    async fn negotiate_capture_only(restore_token: Option<&str>) -> Result<Self, Error> {
        let screencast: Screencast<'static> =
            Screencast::new().await.map_err(portal_error(SCREENCAST))?;
        let session = screencast
            .create_session()
            .await
            .map_err(portal_error(SCREENCAST))?;

        screencast
            .select_sources(
                &session,
                CursorMode::Embedded,
                SourceType::Monitor.into(),
                false,
                // Persistence is allowed here, unlike on a remote-desktop
                // session: this session is the screencast's own.
                restore_token,
                PersistMode::Application,
            )
            .await
            .map_err(portal_error(SCREENCAST))?
            .response()
            .map_err(portal_error(SCREENCAST))?;

        let streams = screencast
            .start(&session, None)
            .await
            .map_err(portal_error(SCREENCAST))?
            .response()
            .map_err(portal_error(SCREENCAST))?;

        if let Some(token) = streams.restore_token() {
            store_restore_token(token);
        }

        let stream = first_stream(Some(streams.streams()))?;

        let fd = screencast
            .open_pipe_wire_remote(&session)
            .await
            .map_err(portal_error(SCREENCAST))?;

        let node_id = stream.pipe_wire_node_id();
        let mut frames = FrameSource::start(fd, node_id)?;
        let stream_size = frames.size().await?;
        let info = describe(&stream, stream_size, false);

        Ok(Self {
            session: SessionHandle::CaptureOnly(session),
            remote_desktop: None,
            info,
            node_id,
            frames,
        })
    }
}

/// The first stream of a session, or an error saying none was chosen.
fn first_stream(streams: Option<&[Stream]>) -> Result<Stream, Error> {
    streams
        .and_then(|streams| streams.first())
        .cloned()
        .ok_or_else(|| Error::Portal {
            interface: SCREENCAST,
            detail: "the session started without selecting a source".to_string(),
        })
}

/// Describe a session from what the portal and the stream reported.
///
/// The portal describes the desktop in logical coordinates while the stream
/// carries physical pixels — on a fractionally scaled display the two differ
/// (1536x960 logical against 2560x1600 of frames, for example). The ratio between
/// them is the scale factor, and it is the hinge every coordinate conversion
/// turns on.
fn describe(stream: &Stream, stream_size: (u32, u32), input_available: bool) -> CaptureInfo {
    let (width, height) = stream.size().unwrap_or((0, 0));
    let logical_size = (width.max(0) as u32, height.max(0) as u32);

    let scale_factor = if logical_size.0 > 0 && logical_size.1 > 0 {
        stream_size.0 as f64 / logical_size.0 as f64
    } else {
        1.0
    };

    CaptureInfo {
        logical_size,
        stream_size,
        origin: stream.position().unwrap_or((0, 0)),
        scale_factor,
        // The pointer's position needs cursor metadata, which this build does not
        // read yet; the embedded cursor is at least visible in the frames.
        pointer: None,
        input_available,
    }
}

/// Map a portal failure onto an error that says what to do about it.
fn portal_error(interface: &'static str) -> impl Fn(ashpd::Error) -> Error {
    move |error| match error {
        ashpd::Error::PortalNotFound(name) => Error::PortalInterfaceUnavailable {
            interface,
            detail: format!("no portal frontend provides {name}"),
        },
        ashpd::Error::RequiresVersion(required, current) => Error::PortalInterfaceUnavailable {
            interface,
            detail: format!("the interface is version {current}, but version {required} is needed"),
        },
        ashpd::Error::Response(ashpd::desktop::ResponseError::Cancelled) => Error::ConsentDenied,
        other => Error::Portal {
            interface,
            detail: other.to_string(),
        },
    }
}

/// Where the portal's consent token is kept between runs.
///
/// The token is what the portal uses to remember the user's choice, so losing it
/// costs a consent prompt, not correctness.
fn restore_token_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state"))
        })?;
    Some(base.join("computer-use").join("restore-token"))
}

/// Read the stored consent token, if there is one.
fn load_restore_token() -> Option<String> {
    let path = restore_token_path()?;
    let token = std::fs::read_to_string(path).ok()?;
    let token = token.trim();
    (!token.is_empty()).then(|| token.to_string())
}

/// Store the consent token so the next run does not have to ask.
fn store_restore_token(token: &str) {
    let Some(path) = restore_token_path() else {
        return;
    };
    if let Some(parent) = path.parent()
        && let Err(error) = std::fs::create_dir_all(parent)
    {
        tracing::warn!("could not create {}: {error}", parent.display());
        return;
    }
    if let Err(error) = std::fs::write(&path, token) {
        tracing::warn!(
            "could not store the consent token at {}: {error}",
            path.display()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_token_path_lives_under_the_state_directory() {
        if let Some(path) = restore_token_path() {
            assert!(path.ends_with("computer-use/restore-token"), "{path:?}");
        }
    }

    #[test]
    fn a_missing_interface_is_reported_as_a_portal_interface_problem() {
        let name = "org.freedesktop.portal.ScreenCast"
            .try_into()
            .expect("a valid name");
        let error = portal_error(SCREENCAST)(ashpd::Error::PortalNotFound(name));
        assert!(
            matches!(error, Error::PortalInterfaceUnavailable { .. }),
            "unexpected error: {error:?}"
        );
        assert!(error.to_string().contains("ScreenCast"), "{error}");
    }

    #[test]
    fn a_cancelled_prompt_is_reported_as_a_denied_consent() {
        let error = portal_error(SCREENCAST)(ashpd::Error::Response(
            ashpd::desktop::ResponseError::Cancelled,
        ));
        assert!(
            matches!(error, Error::ConsentDenied),
            "unexpected error: {error:?}"
        );
    }

    #[test]
    fn only_a_missing_remote_desktop_triggers_the_capture_only_fallback() {
        let remote_desktop_missing = Error::PortalInterfaceUnavailable {
            interface: REMOTE_DESKTOP,
            detail: String::new(),
        };
        assert!(remote_desktop_missing.is_input_unsupported());

        // A missing screencast means there is nothing to fall back to.
        let screencast_missing = Error::PortalInterfaceUnavailable {
            interface: SCREENCAST,
            detail: String::new(),
        };
        assert!(!screencast_missing.is_input_unsupported());

        // And a denied prompt is not a capability gap.
        assert!(!Error::ConsentDenied.is_input_unsupported());
    }
}
