# computer-use

An MCP server that lets an AI agent observe and control a Wayland Linux desktop.

An agent connects over stdio and calls the server's tools to capture the screen
and to move the pointer, click, scroll, type, and send keyboard shortcuts.
Capture and input both go through the XDG Desktop Portal, so the user consents
once and the desktop is never driven invisibly.

**Status:** the first milestone is complete. An agent can capture the screen,
read the display geometry, and drive the desktop — moving the pointer, clicking,
dragging, holding, scrolling, typing, and sending shortcuts — through one
consented portal session. GNOME and KDE support capture and input; wlroots
compositors are capture-only, for the reason in the
[compositor support](#compositor-support) matrix.

## Requirements

A live graphical session. `computer-use` talks to the compositor through the
portal, so it must run inside the user's session, not as a system service. That
means:

- A Wayland session, with `WAYLAND_DISPLAY`, `XDG_RUNTIME_DIR`, and
  `DBUS_SESSION_BUS_ADDRESS` set.
- `xdg-desktop-portal`, plus the backend for your compositor —
  `xdg-desktop-portal-gnome`, `xdg-desktop-portal-kde`, or
  `xdg-desktop-portal-wlr`.
- PipeWire.
- Building additionally needs the PipeWire development headers
  (`pipewire-devel` on Fedora, `libpipewire-0.3-dev` on Debian and Ubuntu).

Starting outside a session fails immediately, naming the variable that is
missing.

## Build

```sh
cargo build --release
```

Build in release mode if you care about speed. A screenshot costs about **57 ms**
optimised on a 2560x1600 desktop, and roughly **2.5 seconds** unoptimised — the
resampling and PNG encoding are tight numeric loops that optimisation transforms.
Measuring a debug build will suggest the tool is far slower than it is.

The `preview` cargo feature is on by default and will carry the `--preview`
window's GUI dependencies. Headless builds can drop it:

```sh
cargo build --release --no-default-features
```

## Use

`computer-use` is an MCP server that speaks the protocol over stdin and stdout,
so an MCP client launches it rather than a person. A client configuration looks
like this:

```json
{
  "mcpServers": {
    "computer-use": {
      "command": "computer-use",
      "args": []
    }
  }
}
```

### Options

| Flag | Default | Meaning |
| --- | --- | --- |
| `--preview` | off | Open a window showing the frames the model sees. Needs the `preview` feature. |
| `--max-dimension <PIXELS>` | `1568` | Long-edge cap applied to screenshots before encoding. |
| `--format <png\|jpeg>` | `png` | Image format used for screenshots. |

### Tools

The tool names follow the [Claude computer
use](https://docs.claude.com/en/docs/agents-and-tools/tool-use/computer-use-tool)
toolset, so a model that knows that vocabulary transfers to this one. The
coordinates do not follow it — see [Coordinates](#coordinates).

| Tool | Purpose |
| --- | --- |
| `screenshot` | The desktop as an image, plus the metadata needed to act on what it shows. |
| `zoom` | One region of the desktop at full resolution, for text a full screenshot makes illegible. |
| `get_display_info` | The captured region's size and position, the stream size, the scale factor, the pointer position, and whether input injection is available here. |
| `cursor_position` | Where the pointer is, in logical desktop coordinates. |
| `mouse_move` | Move the pointer to `coordinate`. |
| `left_click` | Click the primary button, optionally at a `coordinate` and with `modifiers`. |
| `right_click` | Click the secondary button, optionally at a `coordinate` and with `modifiers`. |
| `middle_click` | Click the middle button, optionally at a `coordinate` and with `modifiers`. |
| `double_click` | Double-click the primary button, optionally at a `coordinate`. |
| `triple_click` | Triple-click the primary button, optionally at a `coordinate`. |
| `left_click_drag` | Drag with the primary button from `start_coordinate` to `coordinate`. |
| `left_mouse_down` | Press the primary button and leave it down. |
| `left_mouse_up` | Release the primary button. |
| `scroll` | Scroll `scroll_amount` wheel steps in a `scroll_direction`, optionally at a `coordinate`. |
| `type` | Type a UTF-8 string into whatever currently has focus. |
| `key` | Press a key or combination, `repeat` times. |
| `hold_key` | Hold a key or combination for `duration` seconds. |
| `wait` | Pause for `duration` seconds. |
| `wait_for_change` | Wait until the screen changes, or until `timeout` seconds pass. |

`screenshot` and `zoom` both take `max_dimension` (long-edge cap, default 1568),
`format` (`png` by default, or `jpeg`) and `quality` for JPEG. `--max-dimension`
and `--format` set the defaults for a whole session, so a client need not repeat
them on every call. The two differ in what the cap is spent on: `screenshot`
reduces the whole desktop to fit it, while `zoom` crops `region` first and then
reduces, so a small area comes back at full resolution.

A screenshot resamples and compresses, which is exactly the work that
optimisation transforms: the same call takes about 2.5 s in a debug build and
about 57 ms in a release one. Benchmark a release build, or the numbers will
mislead by an order of magnitude.

`mouse_move` takes the coordinates `get_display_info` and a screenshot's
metadata report, so a position read off an image is passed here unchanged. The
metadata's `image_to_logical_scale` is what maps the two.

**Held and drag interactions.** `left_click_drag` presses at
`start_coordinate`, moves to `coordinate` in steps so applications that wait for
movement still see a drag, and releases. When one gesture is not enough — a
freehand path, or a press that has to survive several calls — use
`left_mouse_down`, move the pointer with `mouse_move`, then `left_mouse_up`.

**Waiting.** Prefer `wait_for_change` to `wait` whenever the thing being waited
for is visible: it returns as soon as the screen changes, which removes the race
a fixed delay leaves. `min_change` is the fraction of sampled pixels that must
differ, defaulting to a value above a blinking caret's footprint. Use `wait` for
a delay, or when nothing on screen will change.

### Keys, combinations, and modifiers

`key` and `hold_key` write a shortcut the way it is spoken: `ctrl+shift+t`,
`alt+F2`, `super`, `ctrl+plus`. Names are case-insensitive. The modifiers are
`ctrl` (or `control`), `shift`, `alt` (or `option`), and `super` (or `meta`,
`win`, `cmd`). A bare modifier is a key in its own right, so `super` alone opens
the overview, and `key: shift` is meaningful.

The key itself is a single character, a named key, or `f1` through `f35`. The
named keys are `enter`, `tab`, `escape`, `space`, `backspace`, `delete`,
`insert`, `home`, `end`, `pageup`, `pagedown`, `up`, `down`, `left`, `right`,
`menu`, `pause`, `print`, `scrolllock`, and `capslock`. Punctuation that would be
confusing as a literal — `plus`, `minus`, `comma`, `period`, `slash`,
`semicolon`, `apostrophe`, `grave`, `bracketleft`, `bracketright`, `backslash`,
`equal` — is written by name, because a literal `+` would read as a separator.

`key` accepts `repeat`, which presses the whole combination that many times —
the way to step through a form or a list with `tab` or an arrow key. `hold_key`
presses and holds for `duration` seconds instead, which is what drives key
auto-repeat; that is how a held arrow key scrolls a list continuously.

The click, drag, and scroll tools take `modifiers` separately, holding them for
that action alone. `left_click` with `modifiers: shift` extends a selection and
with `modifiers: ctrl` toggles one item in a multi-select, and a modifier is
released even when the action fails, so nothing stays stuck down.

`type` is deliberately separate from `key`: typing a string is layout- and
Unicode-sensitive where a shortcut is not. Latin-1 characters map to their own
keysyms and everything else uses the Unicode convention, so an agent can type
text its keyboard layout could not produce.

### Compositor support

| Compositor | Capture | Input |
| --- | --- | --- |
| GNOME | yes | yes |
| KDE | yes | yes |
| wlroots (Sway, Hyprland, river) | yes | no |

The wlroots gap is not a limitation of this project but of its portal backend:
`xdg-desktop-portal-wlr` implements `ScreenCast` and not `RemoteDesktop`, so
there is no portal path for input. Capture keeps working there — the session
falls back to screen casting alone — and the input tools say so when called,
rather than failing with a D-Bus error. The `InputBackend` trait exists so that a
virtual-input or `uinput` backend can close the gap later.

### Coordinates

Three coordinate spaces are in play, and mixing them up is the classic way to
click the wrong thing:

- **Logical desktop coordinates** — what the input tools accept, and what the
  metadata reports positions in.
- **Stream pixels** — what the compositor delivers, which differs from logical
  coordinates whenever the desktop is scaled (1536x960 logical against 2560x1600
  stream pixels here).
- **Image pixels** — what `screenshot` actually returns, which is the stream
  reduced to fit `max_dimension`.

Only the last of those can be measured by looking at the image, so the metadata
carries `image_to_logical_scale`: multiply a position in the returned image by it
to get the coordinates to pass to the input tools. The conversion is exact at any
reduction, and the image is never enlarged.

The input tools take those logical coordinates and scale them by the scale factor
to reach the space the portal's absolute pointer motion expects.

**Known limitation: part of a scaled desktop is out of the portal's reach.**
GNOME's `RemoteDesktop` portal validates an absolute pointer position against the
*logical* desktop size while interpreting it in the *stream's* pixels, so under a
scale factor above 1 it refuses anything past `logical_size / scale_factor` — on
a 1.667-scaled 1536x960 desktop, everything beyond logical (921, 576). That is a
bug in the portal, not here; the input tools report it as an error naming the
position rather than moving the pointer somewhere else. At a scale factor of 1.0
the whole desktop is reachable.

The tool names follow the computer use vocabulary, but the coordinates do not:
that toolset takes positions in the pixel space of the screenshot it returns,
while every coordinate here is a logical desktop coordinate. A position read from
a screenshot's metadata is passed to `left_click` and the other input tools
unchanged.

**The pointer position is where this server last put it, not where it actually
is.** The portal's RemoteDesktop interface does not report the pointer, so
`cursor_position` and the metadata's `pointer` field are at best the last position
commanded. Before anything has moved the pointer they report that it is unknown
rather than guessing.

### The preview window

`--preview` opens a window showing the frames the model is looking at, so the
desktop is never driven invisibly. It is display-only: it takes no input and
offers no controls, because a second way to drive the same desktop would be one
path too many.

The window needs the `preview` cargo feature, which is on by default. A build made
with `--no-default-features` refuses `--preview` and says why. Closing the window
does not stop the server or end the session — the window is a view of the session,
not its owner.

**Known limitation: the preview is itself captured.** Wayland offers no reliable
way to keep one window out of a portal capture, so the preview appears in what the
model sees — a mirror of a mirror. It is harmless, but it clutters the model's
view, so the recommendation is to put the preview on an output other than the one
being captured, or to keep it small. Excluding it is not possible today and is
tracked as a deferred item in the
[requirements](docs/requirements.md).

### Consent

Capture and input are requested on a single portal session, so the user approves
both at once rather than being asked twice. The answer is persisted through the
portal and the resulting token is kept under
`$XDG_STATE_HOME/computer-use/restore-token`, so later runs do not prompt again.
If the user revokes it, the portal rejects the stored token and the server asks
anew.

### Checking capture without an MCP client

`examples/dump_frame.rs` captures a single frame and writes it out as a PNG,
which is the quickest way to confirm that the session, the consent prompt, and
the pixel conversion all work:

```sh
cargo run --example dump_frame -- /tmp/frame.png
```

The first run asks for permission to share the screen; later runs do not.

The run logs the pixel layout and buffer type it negotiated. `DataType::MemFd`
means shared memory, which is what the conversion reads directly. A
`DataType::DmaBuf` reading means the compositor would not fall back to shared
memory: capture then stops with an error naming it, rather than producing
frames.

`examples/reference_screenshot.rs` instead takes a screenshot through the
portal's own `Screenshot` interface, which has GNOME write the PNG itself. That
makes it an independent reference for checking the capture's colours, which is
how the pixel layout was confirmed.

### Logging

Logs go to **stderr only** — stdout is the MCP channel, and anything else there
would corrupt the protocol. Set `COMPUTER_USE_LOG` to change the filter, which
defaults to `info`:

```sh
COMPUTER_USE_LOG=debug computer-use
```

## Layout

```
src/
├── lib.rs         the library the binary, examples, and tests share
├── main.rs        entry point: flags, session preflight, runtime setup
├── cli.rs         command-line flags
├── error.rs       the error type everything reports through
├── logging.rs     stderr-only logging setup
├── session.rs     graphical-session checks
├── mcp.rs         the MCP server and its tool router
├── preview.rs     the --preview window and the pump that feeds it
├── tools.rs       the tools: encoding, cropping, coordinates, arguments
└── backend/
    ├── capture.rs       the CaptureBackend trait and its value types
    ├── input.rs         the InputBackend trait
    ├── keys.rs          text and key names mapped to X11 keysyms
    ├── portal.rs        the portal session: consent, devices, sources, and input
    ├── frame_source.rs  the PipeWire thread that turns buffers into frames
    └── convert.rs       pixel layout conversion to packed RGB
examples/
├── dump_frame.rs           capture one frame to a PNG
└── reference_screenshot.rs take a screenshot through the portal, as a reference
```

Capture and input sit behind traits so another windowing system or platform can
be added without touching the MCP tools.

## Documentation

The project's requirements, decisions, and planned work live under
[`docs/`](docs/index.md). The [backlog](docs/dev/backlog/index.md) is the place
to look for what is being built and why.
