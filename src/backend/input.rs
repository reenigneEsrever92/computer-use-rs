//! The input backend contract.
//!
//! The trait is deliberately primitive: it moves the pointer, presses and
//! releases a button, scrolls, and presses and releases a key. Everything an agent
//! actually asks for — a click, a drag, a shortcut, a key held for a while — is
//! composed from those in the free functions below, so the composition lives in
//! one place, is testable without a session, and no backend has to reimplement it.

use std::time::Duration;

use super::capture::BoxFuture;
use super::keys::{self, Combination, Keysym};
use crate::error::Error;

/// A pointer button, named rather than numbered so callers never deal in evdev
/// button codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Button {
    /// The primary button.
    Left,
    /// The middle button.
    Middle,
    /// The secondary button.
    Right,
}

/// The axis a scroll applies to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Axis {
    /// Up and down.
    Vertical,
    /// Left and right.
    Horizontal,
}

/// Whether an event is a press or a release.
///
/// One type for buttons and keys both, matching the portal, whose
/// `NotifyPointerButton` and `NotifyKeyboardKeysym` take the same state. A press
/// that is not followed by a release is a *hold*, which is what drags, ranges,
/// and auto-repeat need and what an action-shaped API cannot express.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KeyState {
    /// The button or key goes down and stays down until released.
    Pressed,
    /// The button or key comes back up.
    Released,
}

/// A way to act on the desktop.
///
/// Coordinates are always absolute and always in logical desktop coordinates,
/// the same space [`super::capture::CaptureInfo`] reports in.
pub trait InputBackend: Send + Sync + 'static {
    /// Short name of the implementation, used in diagnostics.
    fn name(&self) -> &'static str;

    /// Establish the session, prompting for consent if the platform requires it.
    ///
    /// Idempotent. Where the platform allows it, this shares the consent of the
    /// capture session rather than prompting a second time.
    fn connect(&self) -> BoxFuture<'_, Result<(), Error>>;

    /// Move the pointer to an absolute position.
    fn pointer_move(&self, x: f64, y: f64) -> BoxFuture<'_, Result<(), Error>>;

    /// Press or release a pointer button where the pointer currently is.
    fn pointer_button(&self, button: Button, state: KeyState) -> BoxFuture<'_, Result<(), Error>>;

    /// Scroll by `amount` along `axis`.
    fn pointer_scroll(&self, axis: Axis, amount: f64) -> BoxFuture<'_, Result<(), Error>>;

    /// Press or release a key.
    fn key(&self, keysym: Keysym, state: KeyState) -> BoxFuture<'_, Result<(), Error>>;
}

/// How many intermediate moves a drag makes between its two ends.
///
/// A single jump can arrive as one motion event, and an application that waits
/// for movement before it starts a drag will miss it. Interpolating makes the
/// gesture look like a hand travelling rather than a teleport.
pub const DRAG_STEPS: u32 = 10;

/// Press every keysym in `modifiers`, in the order given.
async fn press_modifiers(input: &dyn InputBackend, modifiers: &[Keysym]) -> Result<(), Error> {
    for keysym in modifiers {
        input.key(*keysym, KeyState::Pressed).await?;
    }
    Ok(())
}

/// Release every keysym in `modifiers`, in reverse.
///
/// Reverse is the order a hand lifts modifiers off, and it keeps a shortcut that
/// opens a new window from having that window receive the modifiers still down.
async fn release_modifiers(input: &dyn InputBackend, modifiers: &[Keysym]) -> Result<(), Error> {
    for keysym in modifiers.iter().rev() {
        input.key(*keysym, KeyState::Released).await?;
    }
    Ok(())
}

/// Hold `modifiers` down around `action`, releasing them whatever happens.
///
/// The release happens even when the action fails: a modifier left down would
/// turn every later call into part of a shortcut, which can be a long way from
/// the failure that caused it.
async fn with_modifiers<F, T>(
    input: &dyn InputBackend,
    modifiers: &[Keysym],
    action: F,
) -> Result<T, Error>
where
    F: std::future::Future<Output = Result<T, Error>>,
{
    press_modifiers(input, modifiers).await?;
    let outcome = action.await;
    let released = release_modifiers(input, modifiers).await;

    match (outcome, released) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(error), _) => Err(error),
        (Ok(_), Err(error)) => Err(error),
    }
}

/// Click `button` `count` times, holding `modifiers` for the gesture.
///
/// Modifiers are held only for the click, which is what makes this a shift-click
/// rather than a click with shift stuck down afterwards.
pub async fn click(
    input: &dyn InputBackend,
    button: Button,
    count: u8,
    modifiers: &[Keysym],
) -> Result<(), Error> {
    with_modifiers(input, modifiers, async {
        for _ in 0..count.max(1) {
            input.pointer_button(button, KeyState::Pressed).await?;
            input.pointer_button(button, KeyState::Released).await?;
        }
        Ok(())
    })
    .await
}

/// Move to `from`, press the primary button, drag to `to`, and release.
pub async fn drag(
    input: &dyn InputBackend,
    from: (f64, f64),
    to: (f64, f64),
    modifiers: &[Keysym],
) -> Result<(), Error> {
    with_modifiers(input, modifiers, async {
        input.pointer_move(from.0, from.1).await?;
        input
            .pointer_button(Button::Left, KeyState::Pressed)
            .await?;

        for step in 1..=DRAG_STEPS {
            let progress = step as f64 / DRAG_STEPS as f64;
            let x = from.0 + (to.0 - from.0) * progress;
            let y = from.1 + (to.1 - from.1) * progress;
            input.pointer_move(x, y).await?;
        }

        input.pointer_button(Button::Left, KeyState::Released).await
    })
    .await
}

/// Scroll by `amount` along `axis`, moving to `at` first when one is given.
pub async fn scroll(
    input: &dyn InputBackend,
    axis: Axis,
    amount: f64,
    at: Option<(f64, f64)>,
    modifiers: &[Keysym],
) -> Result<(), Error> {
    with_modifiers(input, modifiers, async {
        if let Some((x, y)) = at {
            input.pointer_move(x, y).await?;
        }
        input.pointer_scroll(axis, amount).await
    })
    .await
}

/// Type a string, one character at a time.
pub async fn type_text(input: &dyn InputBackend, text: &str) -> Result<(), Error> {
    for keysym in keys::text_keysyms(text) {
        input.key(keysym, KeyState::Pressed).await?;
        input.key(keysym, KeyState::Released).await?;
    }
    Ok(())
}

/// Press `combination` `repeat` times, its modifiers and all.
pub async fn press_combination(
    input: &dyn InputBackend,
    combination: &Combination,
    repeat: u8,
) -> Result<(), Error> {
    for _ in 0..repeat.max(1) {
        with_modifiers(input, &combination.modifiers, async {
            input.key(combination.key, KeyState::Pressed).await?;
            input.key(combination.key, KeyState::Released).await
        })
        .await?;
    }
    Ok(())
}

/// Hold `combination` down for `duration`, then let it up.
///
/// Held rather than tapped because that is what drives key auto-repeat: the
/// compositor turns one press and a long wait into the stream of repeats that a
/// held arrow key produces.
pub async fn hold_keys(
    input: &dyn InputBackend,
    combination: &Combination,
    duration: Duration,
) -> Result<(), Error> {
    with_modifiers(input, &combination.modifiers, async {
        input.key(combination.key, KeyState::Pressed).await?;
        tokio::time::sleep(duration).await;
        input.key(combination.key, KeyState::Released).await
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::keys::{CONTROL, SHIFT};
    use std::sync::Mutex;

    #[derive(Debug, Clone, PartialEq)]
    enum Event {
        Move(f64, f64),
        Button(Button, KeyState),
        Scroll(Axis, f64),
        Key(Keysym, KeyState),
    }

    /// A backend that records what it is asked to do, and can be told to fail on
    /// one event so the cleanup path can be exercised.
    #[derive(Default)]
    struct Recorder {
        events: Mutex<Vec<Event>>,
        fail_on: Option<Event>,
    }

    impl Recorder {
        fn failing_on(event: Event) -> Self {
            Self {
                events: Mutex::new(Vec::new()),
                fail_on: Some(event),
            }
        }

        fn record(&self, event: Event) -> Result<(), Error> {
            let fails = self.fail_on.as_ref() == Some(&event);
            self.events.lock().expect("not poisoned").push(event);
            if fails {
                Err(Error::Input {
                    detail: "the recorder was told to fail".to_string(),
                })
            } else {
                Ok(())
            }
        }

        fn events(&self) -> Vec<Event> {
            self.events.lock().expect("not poisoned").clone()
        }
    }

    impl InputBackend for Recorder {
        fn name(&self) -> &'static str {
            "recorder"
        }

        fn connect(&self) -> BoxFuture<'_, Result<(), Error>> {
            Box::pin(async { Ok(()) })
        }

        fn pointer_move(&self, x: f64, y: f64) -> BoxFuture<'_, Result<(), Error>> {
            Box::pin(async move { self.record(Event::Move(x, y)) })
        }

        fn pointer_button(
            &self,
            button: Button,
            state: KeyState,
        ) -> BoxFuture<'_, Result<(), Error>> {
            Box::pin(async move { self.record(Event::Button(button, state)) })
        }

        fn pointer_scroll(&self, axis: Axis, amount: f64) -> BoxFuture<'_, Result<(), Error>> {
            Box::pin(async move { self.record(Event::Scroll(axis, amount)) })
        }

        fn key(&self, keysym: Keysym, state: KeyState) -> BoxFuture<'_, Result<(), Error>> {
            Box::pin(async move { self.record(Event::Key(keysym, state)) })
        }
    }

    #[tokio::test]
    async fn a_click_presses_and_releases_with_its_modifiers_around_it() {
        let backend = Recorder::default();
        click(&backend, Button::Left, 2, &[CONTROL])
            .await
            .expect("a click");

        assert_eq!(
            backend.events(),
            vec![
                Event::Key(CONTROL, KeyState::Pressed),
                Event::Button(Button::Left, KeyState::Pressed),
                Event::Button(Button::Left, KeyState::Released),
                Event::Button(Button::Left, KeyState::Pressed),
                Event::Button(Button::Left, KeyState::Released),
                Event::Key(CONTROL, KeyState::Released),
            ]
        );
    }

    #[tokio::test]
    async fn modifiers_come_off_even_when_the_action_fails() {
        let backend = Recorder::failing_on(Event::Button(Button::Left, KeyState::Pressed));
        let result = click(&backend, Button::Left, 1, &[SHIFT, CONTROL]).await;
        assert!(result.is_err(), "the failure should reach the caller");

        let events = backend.events();
        // Released in reverse, so the last one off is the first one put on.
        assert_eq!(events.last(), Some(&Event::Key(SHIFT, KeyState::Released)));
        assert!(events.contains(&Event::Key(CONTROL, KeyState::Released)));
    }

    #[tokio::test]
    async fn a_drag_moves_to_the_start_presses_and_releases_at_the_end() {
        let backend = Recorder::default();
        drag(&backend, (10.0, 20.0), (110.0, 220.0), &[])
            .await
            .expect("a drag");

        let events = backend.events();
        assert_eq!(events.first(), Some(&Event::Move(10.0, 20.0)));
        assert_eq!(
            events.get(1),
            Some(&Event::Button(Button::Left, KeyState::Pressed))
        );
        assert_eq!(
            events.last(),
            Some(&Event::Button(Button::Left, KeyState::Released))
        );

        // The end is reached by movement, not a jump, so the drag is recognised.
        assert!(events.contains(&Event::Move(110.0, 220.0)));
        let moves = events
            .iter()
            .filter(|event| matches!(event, Event::Move(..)))
            .count();
        assert_eq!(moves, DRAG_STEPS as usize + 1);
    }

    #[tokio::test]
    async fn scrolling_moves_first_when_a_position_is_given() {
        let backend = Recorder::default();
        scroll(&backend, Axis::Vertical, 3.0, Some((5.0, 6.0)), &[])
            .await
            .expect("a scroll");

        assert_eq!(
            backend.events(),
            vec![Event::Move(5.0, 6.0), Event::Scroll(Axis::Vertical, 3.0)]
        );
    }

    #[tokio::test]
    async fn typing_presses_each_character_in_turn() {
        let backend = Recorder::default();
        type_text(&backend, "aé").await.expect("typing");

        assert_eq!(
            backend.events(),
            vec![
                Event::Key(0x61, KeyState::Pressed),
                Event::Key(0x61, KeyState::Released),
                Event::Key(0xe9, KeyState::Pressed),
                Event::Key(0xe9, KeyState::Released),
            ]
        );
    }

    #[tokio::test]
    async fn a_combination_is_pressed_the_requested_number_of_times() {
        let backend = Recorder::default();
        let combination = keys::parse_combination("ctrl+l").expect("a combination");
        press_combination(&backend, &combination, 3)
            .await
            .expect("pressing");

        let presses = backend
            .events()
            .iter()
            .filter(|event| matches!(event, Event::Key(0x6c, KeyState::Pressed)))
            .count();
        assert_eq!(presses, 3);
    }

    #[tokio::test]
    async fn a_held_key_is_pressed_and_then_released() {
        let backend = Recorder::default();
        let combination = keys::parse_combination("down").expect("a combination");
        hold_keys(&backend, &combination, Duration::from_millis(0))
            .await
            .expect("holding");

        assert_eq!(
            backend.events(),
            vec![
                Event::Key(0xff54, KeyState::Pressed),
                Event::Key(0xff54, KeyState::Released),
            ]
        );
    }
}
