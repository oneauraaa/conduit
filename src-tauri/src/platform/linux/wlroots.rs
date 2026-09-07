//! Synthetic input through `wlr-virtual-pointer` and `virtual-keyboard`.
//!
//! ## Why this exists beside the portal
//!
//! [`super::portal`] is the *standard* way to inject input on Wayland, and on a
//! large part of the desktop it simply is not there. `RemoteDesktop` is
//! implemented by the GNOME and KDE portal backends and by nobody else:
//! `xdg-desktop-portal-hyprland` implements `ScreenCast` and stops, and there is
//! no wlroots backend that goes further. On Hyprland, sway, river and the rest,
//! asking the portal for a remote-desktop session earns
//!
//! ```text
//! org.freedesktop.DBus.Error.UnknownMethod: No such interface
//! "org.freedesktop.portal.RemoteDesktop"
//! ```
//!
//! — not a denial the user can fix by installing a package, because no package
//! provides it. Without this module conduit cannot move the pointer or press a
//! key on any of those compositors, which is most of the reason someone runs it.
//!
//! What those compositors offer instead is these two protocols. They are not a
//! workaround: `zwlr_virtual_pointer_manager_v1` and
//! `zwp_virtual_keyboard_manager_v1` are how a wlroots session expects a remote
//! desktop, an on-screen keyboard or an accessibility tool to inject events.
//!
//! ## Why there is no prompt
//!
//! There is nothing to prompt for. Unlike the portal, which brokers consent
//! because it hands out a capability the compositor cannot scope, these
//! protocols are advertised in the registry and a client that can see them is
//! already trusted by the compositor — the socket in `WAYLAND_DISPLAY` *is* the
//! grant. Hyprland restricts them through its own permission rules if the user
//! wants that.
//!
//! This is why [`super::sink`] prefers this route where it exists. It costs the
//! user nothing per launch, whereas the portal's dialog cannot be made
//! permanent. conduit still asks for the portal — screen *capture* has no
//! wlroots shortcut and `ScreenCast` is the only route to it — so the prompt
//! does not disappear, it just stops gating the pointer.
//!
//! ## Why the keyboard builds its own keymap
//!
//! `zwp_virtual_keyboard_v1` speaks *keycodes*, and conduit's whole Linux input
//! layer speaks *keysyms* — see the header of [`super::keycodes`] for why that
//! is the correct choice and not an accident. A keycode is meaningless without
//! the keymap it is read through, so rather than guess at the user's layout and
//! type `"` where an agent asked for `@`, this module uploads a keymap of its
//! own in which every keysym conduit needs sits at level 1 of a keycode of its
//! own.
//!
//! That inverts the usual problem. Nothing needs Shift to reach a capital or a
//! symbol, because the keysym *is* the unshifted level, so `type_text` is exact
//! on a Turkish, Dvorak or Japanese layout for the same reason the portal path
//! is. The four real modifiers keep dedicated keycodes with a `modifier_map`, so
//! a shortcut like `cmd+c` still arrives as a genuine Control press that
//! applications see as one.

use std::collections::HashMap;
use std::io::Write;
use std::os::fd::{AsFd, FromRawFd, OwnedFd};
use std::sync::OnceLock;
use std::time::Instant;

use parking_lot::Mutex;
use wayland_client::protocol::{wl_output, wl_registry, wl_seat};
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle, delegate_noop};
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::{
    zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1,
    zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1,
};
use wayland_protocols_wlr::virtual_pointer::v1::client::{
    zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1,
    zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1,
};

use super::keycodes;
use crate::platform::types::display_at;

/// `WL_KEYBOARD_KEYMAP_FORMAT_XKB_V1`.
const KEYMAP_FORMAT_XKB_V1: u32 = 1;

/// XKB reserves everything below 8; evdev codes are `xkb - 8`, so this is
/// evdev 0 and the first code a keymap may name.
const XKB_MIN: u32 = 8;
/// The last code an `xkb_keycodes` block may name. 247 usable slots in all.
const XKB_MAX: u32 = 255;

/// The four modifiers get fixed keycodes so a shortcut never has to wait for a
/// keymap upload, and so [`reset_dynamic`] can recycle everything else without
/// ever pulling a held modifier out from under a keypress.
const KC_SHIFT: u32 = XKB_MIN + 1;
const KC_CONTROL: u32 = XKB_MIN + 2;
const KC_ALT: u32 = XKB_MIN + 3;
const KC_SUPER: u32 = XKB_MIN + 4;
/// Where the keysym pool starts, just past the modifiers.
const KC_DYNAMIC: u32 = XKB_MIN + 5;

/* ── the connection ─────────────────────────────────────────── */

/// One output, and where it sits in the compositor's layout.
struct Output {
    output: wl_output::WlOutput,
    x: i32,
    y: i32,
}

/// What the registry roundtrip collects.
#[derive(Default)]
struct Globals {
    pointer_manager: Option<ZwlrVirtualPointerManagerV1>,
    /// The version actually bound. Below 2 there is no per-output pointer, so
    /// absolute motion has to be expressed against the whole layout instead.
    pointer_version: u32,
    keyboard_manager: Option<ZwpVirtualKeyboardManagerV1>,
    seat: Option<wl_seat::WlSeat>,
    outputs: Vec<Output>,
}

/// Everything that needs the Wayland connection, once it is known to be usable.
struct Backend {
    conn: Connection,
    qh: QueueHandle<Globals>,
    /// The queue and the state it dispatches into, together because
    /// `roundtrip` needs both and they are only ever touched as a pair.
    pump: Mutex<(EventQueue<Globals>, Globals)>,
    pointer_manager: ZwlrVirtualPointerManagerV1,
    /// See [`Globals::pointer_version`].
    pointer_version: u32,
    seat: wl_seat::WlSeat,
    keyboard: ZwpVirtualKeyboardV1,
    /// One virtual pointer per output, made on first use. Absolute motion is
    /// expressed relative to the output the pointer was created against, so a
    /// second monitor genuinely needs a second device.
    pointers: Mutex<HashMap<usize, ZwlrVirtualPointerV1>>,
    keymap: Mutex<Keymap>,
}

// None of these four ever send an event conduit acts on. The managers are
// factories, and the virtual devices are write-only by design — the compositor
// has nothing to tell a client that is pretending to be a mouse.
delegate_noop!(Globals: ignore ZwlrVirtualPointerManagerV1);
delegate_noop!(Globals: ignore ZwlrVirtualPointerV1);
delegate_noop!(Globals: ignore ZwpVirtualKeyboardManagerV1);
delegate_noop!(Globals: ignore ZwpVirtualKeyboardV1);
delegate_noop!(Globals: ignore wl_seat::WlSeat);

impl Dispatch<wl_registry::WlRegistry, ()> for Globals {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        let wl_registry::Event::Global { name, interface, version } = event else {
            return;
        };

        match interface.as_str() {
            "zwlr_virtual_pointer_manager_v1" => {
                // Version 2, when the compositor has it: that is the one that
                // added `create_virtual_pointer_with_output`, and asking a
                // version-1 object for it is a protocol error the compositor
                // answers by dropping the connection — silently, from this
                // side, because no request here expects a reply.
                state.pointer_version = version.min(2);
                state.pointer_manager = Some(registry.bind::<ZwlrVirtualPointerManagerV1, _, _>(
                    name,
                    state.pointer_version,
                    qh,
                    (),
                ));
            }
            "zwp_virtual_keyboard_manager_v1" => {
                state.keyboard_manager =
                    Some(registry.bind::<ZwpVirtualKeyboardManagerV1, _, _>(name, 1, qh, ()));
            }
            "wl_seat" => {
                // The first seat. Multi-seat machines exist; a machine with two
                // people driving it is not one an agent should be let loose on
                // anyway, and picking the first is what every other tool does.
                if state.seat.is_none() {
                    state.seat =
                        Some(registry.bind::<wl_seat::WlSeat, _, _>(name, version.min(7), qh, ()));
                }
            }
            "wl_output" => {
                // Version 2 for the `done` event, which is what makes the
                // geometry below safe to read after a single roundtrip.
                let output =
                    registry.bind::<wl_output::WlOutput, _, _>(name, version.min(2), qh, ());
                state.outputs.push(Output { output, x: 0, y: 0 });
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_output::WlOutput, ()> for Globals {
    fn event(
        state: &mut Self,
        output: &wl_output::WlOutput,
        event: wl_output::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // Only the origin. The *size* comes from conduit's own display list
        // rather than from `mode` — `mode` is in physical pixels and this
        // backend's coordinate space is logical, and dividing by an integer
        // `scale` is wrong the moment fractional scaling is on. GDK already
        // reports the logical size correctly, so there is no reason to derive
        // a second, worse answer here.
        if let wl_output::Event::Geometry { x, y, .. } = event {
            if let Some(slot) = state.outputs.iter_mut().find(|o| o.output == *output) {
                slot.x = x;
                slot.y = y;
            }
        }
    }
}

/// Why this route is unusable, if it is.
///
/// Only ever "the compositor does not offer it" — there is no consent step to
/// fail and no session to lose, which is the whole advantage over the portal.
static INIT: OnceLock<Result<Backend, String>> = OnceLock::new();

fn backend() -> Result<&'static Backend, String> {
    INIT.get_or_init(connect).as_ref().map_err(String::clone)
}

/// Whether this machine can inject input this way, without side effects.
///
/// Connecting binds a virtual keyboard, which is harmless and invisible — no
/// dialog, no device shown to the user, nothing typed. That is why the
/// readiness card may call this on a poll, and why [`super::sink`] can decide
/// its route eagerly instead of on the first click.
pub fn available() -> bool {
    backend().is_ok()
}

/// The reason this route is unavailable, for the readiness card.
pub fn unavailable_reason() -> Option<String> {
    backend().err()
}

fn connect() -> Result<Backend, String> {
    let conn = Connection::connect_to_env()
        .map_err(|e| format!("could not open a wayland connection: {e}"))?;

    let display = conn.display();
    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    display.get_registry(&qh, ());

    let mut globals = Globals::default();
    // Twice: the first pass binds the globals the registry advertises, the
    // second collects the events those bindings then emit — an output's
    // geometry arrives only after it has been bound.
    for _ in 0..2 {
        queue
            .roundtrip(&mut globals)
            .map_err(|e| format!("the wayland compositor closed the connection: {e}"))?;
    }

    let missing = |what: &str| {
        format!(
            "this compositor does not offer {what}. conduit can drive the pointer and keyboard \
             through the wlroots virtual-input protocols (hyprland, sway, river) or through the \
             desktop portal's remote-desktop interface (gnome, kde); this desktop appears to \
             offer neither."
        )
    };

    let pointer_manager = globals
        .pointer_manager
        .clone()
        .ok_or_else(|| missing("zwlr_virtual_pointer_manager_v1"))?;
    let keyboard_manager = globals
        .keyboard_manager
        .clone()
        .ok_or_else(|| missing("zwp_virtual_keyboard_manager_v1"))?;
    let seat = globals
        .seat
        .clone()
        .ok_or_else(|| "this compositor advertises no wl_seat, so there is nothing to \
                        inject input into.".to_string())?;

    let keyboard = keyboard_manager.create_virtual_keyboard(&seat, &qh, ());

    let backend = Backend {
        conn: conn.clone(),
        qh,
        pump: Mutex::new((queue, globals)),
        pointer_manager,
        pointer_version: globals.pointer_version,
        seat,
        keyboard,
        pointers: Mutex::new(HashMap::new()),
        keymap: Mutex::new(Keymap::new()),
    };

    // A keyboard with no keymap is a protocol error the moment a key is sent,
    // so the first upload happens here rather than on the first keystroke.
    backend.upload_keymap()?;

    tracing::info!("wlroots virtual input is available; using it instead of the portal");
    Ok(backend)
}

/// Milliseconds since the backend came up.
///
/// The protocol wants a timestamp with millisecond resolution and does not care
/// about its epoch — only that it advances, so that a compositor can tell a
/// double-click from two clicks.
fn now_ms() -> u32 {
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_millis() as u32
}

impl Backend {
    /// Pushes queued requests to the compositor.
    ///
    /// Every public entry point ends in one of these. Without it a request sits
    /// in the client-side buffer until something else happens to flush it,
    /// which for a pointer move means the cursor lands late or not at all.
    fn flush(&self) -> Result<(), String> {
        // The protocol error first. A request the compositor refuses gets no
        // reply — it closes the connection instead — so without this check
        // every later call would quietly do nothing and report success, which
        // is the one thing `input_ok` exists to prevent.
        if let Some(e) = self.conn.protocol_error() {
            return Err(format!(
                "the compositor refused a virtual-input request: {} (interface {}, code {})",
                e.message, e.interface, e.code
            ));
        }
        self.conn
            .flush()
            .map_err(|e| format!("the wayland connection failed: {e}"))
    }

    /// The virtual pointer for the output containing a point, made on demand.
    ///
    /// Absolute motion is normalized against the extent passed with it and
    /// interpreted on the pointer's own output, so the point has to be made
    /// local to that output first. Returning the extent alongside keeps the two
    /// halves of that conversion together.
    fn pointer_for(&self, x: f64, y: f64) -> Result<Target, String> {
        let displays = crate::chrome::cached_displays();
        if displays.is_empty() {
            return Err("conduit does not have the display geometry yet".into());
        }
        // `display_at` falls back to index 0 for a point off every screen, which
        // is the right answer here — a pointer has to go somewhere.
        let display = &displays[display_at(&displays, x, y).min(displays.len() - 1)];

        let pump = self.pump.lock();
        // Matched by origin rather than by index: `screen::displays` sorts its
        // own way and the registry advertises outputs in whatever order it
        // likes, so lining the two lists up positionally would be a coin flip
        // that silently sends every click to the wrong monitor.
        let matched = if self.pointer_version >= 2 {
            pump.1
                .outputs
                .iter()
                .position(|o| f64::from(o.x) == display.x && f64::from(o.y) == display.y)
        } else {
            None
        };

        let mut pointers = self.pointers.lock();
        match matched {
            Some(i) => {
                let pointer = pointers
                    .entry(i)
                    .or_insert_with(|| {
                        self.pointer_manager.create_virtual_pointer_with_output(
                            Some(&self.seat),
                            Some(&pump.1.outputs[i].output),
                            &self.qh,
                            (),
                        )
                    })
                    .clone();
                Ok(Target {
                    pointer,
                    x: x - display.x,
                    y: y - display.y,
                    width: display.width,
                    height: display.height,
                })
            }
            // No output matched. Rather than refuse the click, fall back to a
            // pointer bound to no output, which the compositor maps across the
            // whole layout — exactly right on the overwhelmingly common
            // single-monitor machine, and better than nothing on the rest.
            None => {
                let pointer = pointers
                    .entry(UNBOUND)
                    .or_insert_with(|| {
                        self.pointer_manager
                            .create_virtual_pointer(Some(&self.seat), &self.qh, ())
                    })
                    .clone();
                let min_x = displays.iter().map(|d| d.x).fold(f64::INFINITY, f64::min);
                let min_y = displays.iter().map(|d| d.y).fold(f64::INFINITY, f64::min);
                let max_x = displays
                    .iter()
                    .map(|d| d.x + d.width)
                    .fold(f64::NEG_INFINITY, f64::max);
                let max_y = displays
                    .iter()
                    .map(|d| d.y + d.height)
                    .fold(f64::NEG_INFINITY, f64::max);
                Ok(Target {
                    pointer,
                    x: x - min_x,
                    y: y - min_y,
                    width: max_x - min_x,
                    height: max_y - min_y,
                })
            }
        }
    }
}

/// A pointer and the coordinate space its absolute motion is measured in.
///
/// The two travel together because they are one answer: a position is
/// meaningless without the extent it is normalized against, and the extent
/// differs per output.
struct Target {
    pointer: ZwlrVirtualPointerV1,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

/// The key the unbound, whole-layout pointer is cached under. Not an output
/// index, so it can never collide with one.
const UNBOUND: usize = usize::MAX;

/* ── the surface `sink` calls ───────────────────────────────── */

pub fn pointer_motion_absolute(x: f64, y: f64) -> Result<(), String> {
    let backend = backend()?;
    let target = backend.pointer_for(x, y)?;

    // The extent is the denominator the compositor divides by, so it has to be
    // the output's size in the same units the position is given in. Clamped
    // one short of the extent: a position *equal* to it normalizes to exactly
    // 1.0, which lands on the first pixel of the next output.
    let ex = target.width.max(1.0);
    let ey = target.height.max(1.0);
    target.pointer.motion_absolute(
        now_ms(),
        target.x.clamp(0.0, ex - 1.0).round() as u32,
        target.y.clamp(0.0, ey - 1.0).round() as u32,
        ex.round() as u32,
        ey.round() as u32,
    );
    // Every logical group of pointer events has to be terminated by a frame or
    // wlroots holds them: the cursor does not move until the *next* call
    // happens to flush it, which reads as conduit lagging one action behind.
    target.pointer.frame();
    backend.flush()
}

pub fn pointer_button(button: i32, pressed: bool) -> Result<(), String> {
    let backend = backend()?;
    // Buttons act wherever the pointer already is, and this backend's pointers
    // are per-output, so the button has to go to the same device the last move
    // did. `cursor_position` is conduit's record of that.
    let (x, y) = super::input::cursor_position();
    let target = backend.pointer_for(x, y)?;

    let state = if pressed {
        wayland_client::protocol::wl_pointer::ButtonState::Pressed
    } else {
        wayland_client::protocol::wl_pointer::ButtonState::Released
    };
    target.pointer.button(now_ms(), button as u32, state);
    target.pointer.frame();
    backend.flush()
}

/// Scrolls by whole detents, matching the portal path's units.
///
/// `axis_discrete` carries both a continuous value and a step count because
/// toolkits split on which they honour: the step count is what a compositor
/// turns into a wheel click, and the value is what smooth-scrolling clients
/// read. Sending only one of them makes conduit scroll in some applications and
/// not others, so both go out with consistent magnitudes.
pub fn pointer_axis_discrete(axis: u32, steps: i32) -> Result<(), String> {
    /// The conventional 15° wheel step, in the pixel-ish units the axis event
    /// is measured in.
    const UNITS_PER_DETENT: f64 = 15.0;

    let backend = backend()?;
    let (x, y) = super::input::cursor_position();
    let target = backend.pointer_for(x, y)?;

    let axis = match axis {
        0 => wayland_client::protocol::wl_pointer::Axis::VerticalScroll,
        _ => wayland_client::protocol::wl_pointer::Axis::HorizontalScroll,
    };
    target.pointer.axis_discrete(
        now_ms(),
        axis,
        f64::from(steps) * UNITS_PER_DETENT,
        steps,
    );
    target.pointer.frame();
    backend.flush()
}

pub fn keyboard_keysym(keysym: i32, pressed: bool) -> Result<(), String> {
    let backend = backend()?;

    let (code, upload) = {
        let mut keymap = backend.keymap.lock();
        let code = keymap.keycode_for(keysym, pressed)?;
        (code, keymap.take_dirty())
    };
    if upload {
        backend.upload_keymap()?;
    }

    // The protocol's `key` is an evdev code, which is the xkb keycode less the
    // 8 codes xkb reserves. Sending the xkb code raw shifts every keystroke by
    // eight keys, which types plausible nonsense rather than failing.
    backend
        .keyboard
        .key(now_ms(), code - XKB_MIN, u32::from(pressed));
    backend.flush()
}

/* ── the keymap ─────────────────────────────────────────────── */

/// conduit's own keymap: one keysym per keycode, at level 1.
///
/// See the module header for why this shape. The map grows as new keysyms are
/// asked for and is re-uploaded when it does, which for ordinary text happens
/// only on the first few keystrokes — the seed below covers everything an
/// English keyboard can reach, so the re-upload path exists for the `こんにちは`
/// case rather than the common one.
struct Keymap {
    /// keysym → xkb keycode, for the pool past the modifiers.
    slots: HashMap<i32, u32>,
    /// The next free keycode.
    next: u32,
    /// Whether the compositor's copy is behind.
    dirty: bool,
    /// Keysyms currently held down, so recycling never pulls a keycode out
    /// from under a key that has been pressed but not released.
    held: Vec<i32>,
}

impl Keymap {
    fn new() -> Self {
        let mut map = Keymap {
            slots: HashMap::new(),
            next: KC_DYNAMIC,
            dirty: true,
            held: Vec::new(),
        };
        // Everything a US keyboard can type, seeded up front so that ordinary
        // text never pays for a keymap upload mid-word.
        for sym in 0x20..=0x7e {
            let _ = map.assign(sym);
        }
        // The named keys `keycodes::lookup` can return, minus the modifiers,
        // which have fixed codes. Same reason: `key_press("return")` should not
        // be the call that recompiles a keymap.
        for sym in [
            0xff08, 0xff09, 0xff0d, 0xff1b, 0xff50, 0xff51, 0xff52, 0xff53, 0xff54, 0xff55,
            0xff56, 0xff57, 0xffff,
        ] {
            let _ = map.assign(sym);
        }
        // F1–F12.
        for sym in 0xffbe..=0xffc9 {
            let _ = map.assign(sym);
        }
        map
    }

    /// Whether the compositor needs a fresh copy, clearing the flag.
    fn take_dirty(&mut self) -> bool {
        std::mem::replace(&mut self.dirty, false)
    }

    fn assign(&mut self, keysym: i32) -> Result<u32, String> {
        if let Some(code) = self.slots.get(&keysym) {
            return Ok(*code);
        }
        if self.next > XKB_MAX {
            return Err("the keymap is full".into());
        }
        let code = self.next;
        self.next += 1;
        self.slots.insert(keysym, code);
        self.dirty = true;
        Ok(code)
    }

    /// The keycode that produces `keysym`, adding it to the map if it is new.
    ///
    /// `pressed` is tracked rather than ignored because a full pool has to be
    /// recycled, and the one thing recycling must never do is move a keycode
    /// that a caller is still holding down — that would leave a key stuck for
    /// every application on the machine.
    fn keycode_for(&mut self, keysym: i32, pressed: bool) -> Result<u32, String> {
        let code = match keysym {
            keycodes::SHIFT => KC_SHIFT,
            keycodes::CONTROL => KC_CONTROL,
            keycodes::ALT => KC_ALT,
            keycodes::SUPER => KC_SUPER,
            _ => match self.assign(keysym) {
                Ok(code) => code,
                Err(_) => {
                    // 247 distinct keysyms in one session is a very long stretch
                    // of text in a script with no repeats. Start the pool over
                    // rather than refusing to type.
                    if !self.held.is_empty() {
                        return Err(
                            "the keymap is full while keys are still held down".into()
                        );
                    }
                    self.slots.clear();
                    self.next = KC_DYNAMIC;
                    self.assign(keysym)?
                }
            },
        };

        if pressed {
            self.held.push(keysym);
        } else {
            self.held.retain(|k| *k != keysym);
        }
        Ok(code)
    }

    /// The keymap as xkb's text format.
    ///
    /// `include "complete"` for types and compatibility rather than a
    /// hand-written pair: those files are what give `modifier_map Control` its
    /// meaning, and they ship with xkeyboard-config, which any machine running
    /// a Wayland compositor already has.
    fn to_xkb(&self) -> String {
        let mut codes = String::new();
        let mut symbols = String::new();

        for (name, code, keysym, modifier) in [
            ("shift", KC_SHIFT, "Shift_L", Some("Shift")),
            ("control", KC_CONTROL, "Control_L", Some("Control")),
            ("alt", KC_ALT, "Alt_L", Some("Mod1")),
            ("super", KC_SUPER, "Super_L", Some("Mod4")),
        ] {
            codes.push_str(&format!("    <{name}> = {code};\n"));
            symbols.push_str(&format!("    key <{name}> {{ [ {keysym} ] }};\n"));
            if let Some(m) = modifier {
                symbols.push_str(&format!("    modifier_map {m} {{ <{name}> }};\n"));
            }
        }

        // Sorted, so the same set of keysyms always produces byte-identical
        // text. A keymap that shuffled itself between uploads would be a
        // miserable thing to diff against a compositor log.
        let mut slots: Vec<(&i32, &u32)> = self.slots.iter().collect();
        slots.sort_by_key(|(_, code)| **code);

        for (keysym, code) in slots {
            codes.push_str(&format!("    <k{code}> = {code};\n"));
            // Numeric keysyms, which xkbcommon parses as `0x…` and resolves to
            // the same symbol a name would. Writing them numerically is what
            // lets an arbitrary unicode escape — keysym `0x01000000 + cp`, the
            // thing that makes non-latin text typable at all — go in without a
            // name table conduit would have to carry.
            symbols.push_str(&format!("    key <k{code}> {{ [ 0x{keysym:08x} ] }};\n"));
        }

        format!(
            "xkb_keymap {{\n\
             xkb_keycodes \"conduit\" {{\n\
             \x20   minimum = {XKB_MIN};\n\
             \x20   maximum = {XKB_MAX};\n\
             {codes}\
             }};\n\
             xkb_types \"conduit\" {{ include \"complete\" }};\n\
             xkb_compatibility \"conduit\" {{ include \"complete\" }};\n\
             xkb_symbols \"conduit\" {{\n\
             {symbols}\
             }};\n\
             }};\n"
        )
    }
}

impl Backend {
    /// Hands the compositor the current keymap.
    ///
    /// Through a `memfd` rather than a file in `/tmp`: the compositor mmaps
    /// what it is given, and an anonymous descriptor cannot be read, replaced
    /// or left behind by anything else on the machine.
    fn upload_keymap(&self) -> Result<(), String> {
        let text = self.keymap.lock().to_xkb();
        // The size the compositor mmaps includes the terminating NUL, and
        // xkbcommon reads the buffer as a C string. One byte short and the
        // keymap fails to compile with no useful message on either side.
        let bytes = {
            let mut b = text.into_bytes();
            b.push(0);
            b
        };

        let mut file = memfd(&bytes)?;
        file.flush().map_err(|e| format!("could not write the keymap: {e}"))?;

        self.keyboard.keymap(
            KEYMAP_FORMAT_XKB_V1,
            file.as_fd(),
            bytes.len() as u32,
        );

        // A roundtrip, not a flush: the compositor has to have compiled this
        // before the next key event refers to a code that only exists in it.
        let mut pump = self.pump.lock();
        let (queue, globals) = &mut *pump;
        queue
            .roundtrip(globals)
            .map_err(|e| format!("the compositor rejected the keymap: {e}"))?;
        Ok(())
    }
}

/// An anonymous, memory-backed file holding `bytes`.
fn memfd(bytes: &[u8]) -> Result<std::fs::File, String> {
    let name = c"conduit-keymap";
    // SAFETY: a valid NUL-terminated name and a documented flag; the returned
    // descriptor is checked and immediately given an owner.
    let raw = unsafe { libc::memfd_create(name.as_ptr(), libc::MFD_CLOEXEC) };
    if raw < 0 {
        return Err(format!(
            "could not create a keymap descriptor: {}",
            std::io::Error::last_os_error()
        ));
    }
    // SAFETY: `raw` is a fresh descriptor this function exclusively owns.
    let fd = unsafe { OwnedFd::from_raw_fd(raw) };
    let mut file = std::fs::File::from(fd);
    file.write_all(bytes)
        .map_err(|e| format!("could not write the keymap: {e}"))?;
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The keymap has to compile. Every keystroke this backend sends is a
    /// keycode that means nothing without it, so a keymap that xkbcommon
    /// rejects is not a degraded state — it is a keyboard that types garbage
    /// into whatever the user had focused.
    #[test]
    fn the_seeded_keymap_is_valid_xkb() {
        let text = Keymap::new().to_xkb();
        // The four modifiers, with the mapping that makes `cmd+c` a real
        // Control press rather than a lone unmodified `c`.
        assert!(text.contains("modifier_map Control { <control> };"), "{text}");
        assert!(text.contains("modifier_map Mod4 { <super> };"), "{text}");
        // Printable ASCII is seeded, so ordinary typing never re-uploads.
        assert!(text.contains("[ 0x00000061 ]"), "{text}");
        assert!(text.contains("[ 0x0000007e ]"), "{text}");
    }

    /// The escape that makes non-latin text typable has to survive the trip
    /// into the keymap — this is the property the portal path gets for free
    /// and this one has to construct.
    #[test]
    fn a_unicode_keysym_reaches_the_keymap_numerically() {
        let mut map = Keymap::new();
        let sym = keycodes::keysym_for_char('こ');
        let code = map.keycode_for(sym, true).unwrap();
        assert!(code >= KC_DYNAMIC);
        assert!(map.to_xkb().contains(&format!("[ 0x{sym:08x} ]")));
    }

    /// Modifiers keep fixed codes, so a shortcut never waits on an upload and
    /// recycling can never move one.
    #[test]
    fn modifiers_never_come_from_the_pool() {
        let mut map = Keymap::new();
        for (sym, expected) in [
            (keycodes::SHIFT, KC_SHIFT),
            (keycodes::CONTROL, KC_CONTROL),
            (keycodes::ALT, KC_ALT),
            (keycodes::SUPER, KC_SUPER),
        ] {
            assert_eq!(map.keycode_for(sym, true).unwrap(), expected);
        }
        // Holding all four must not have consumed a single pool slot.
        assert!(map.slots.values().all(|c| *c >= KC_DYNAMIC));
    }

    /// A key that has been pressed and not released must keep its keycode.
    /// Recycling under it would leave that key stuck down system-wide, which
    /// no application recovers from on its own.
    #[test]
    fn a_held_key_blocks_recycling_rather_than_being_moved() {
        let mut map = Keymap::new();
        // Fill the pool with keysyms nothing has seeded.
        let mut sym = 0x0100_0000;
        while map.assign(sym).is_ok() {
            sym += 1;
        }
        map.held.push(0x0100_0000);
        assert!(map.keycode_for(sym + 1, true).is_err());

        // With nothing held, the pool starts over instead of refusing.
        map.held.clear();
        assert!(map.keycode_for(sym + 1, true).is_ok());
    }

    /// The seed exists to keep ordinary typing off the upload path. If it stops
    /// covering ASCII, every English keystroke starts recompiling a keymap.
    #[test]
    fn typing_ascii_never_dirties_the_keymap() {
        let mut map = Keymap::new();
        map.take_dirty();
        for c in "Hello, world! (1+2=3)".chars() {
            map.keycode_for(keycodes::keysym_for_char(c), true).unwrap();
            map.keycode_for(keycodes::keysym_for_char(c), false).unwrap();
        }
        assert!(!map.take_dirty());
    }
}
