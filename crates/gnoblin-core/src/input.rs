//! Input claims: key chords, pointer chords, touchpad gestures, devices and input grabs.
//!
//! A shell takes input from the compositor by claiming it on a seat. The compositor then withholds a claimed chord from every
//! client and tells the shell instead; a claim lasts until its [`Subscription`] is removed or the generation retires, and a
//! reload hands a chord from the old generation to the new one without the compositor ever seeing it claimed twice.
//!
//! ```ignore
//! g.bind("<Super>Return", || { g.spawn("foot").ok(); })?;
//! g.bind_pointer("<Super>Button1", |drag| { /* move a window by drag.dx, drag.dy */ })?;
//! ```
#![allow(unsafe_code)]

use core::ffi::c_void;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::Ordering;

use gnoblin_core_abi::{self as abi, GnoblinClaim, GnoblinClaimEvent, GnoblinStr};

use crate::error::Result;
use crate::events::Slot;
use crate::gnoblin::{CALLS, Gnoblin, Subscription, drop_slot};
use crate::tree::Node;

/// The state a chord changed to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChordState {
    /// The chord went down.
    Press,
    /// The chord was released (a claim made with a grab hears it).
    Release,
    /// The key repeats while held.
    Repeat,
}

/// A claimed chord changed state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChordEvent {
    /// The chord as the compositor spells it after canonicalising what the shell claimed.
    pub chord: String,
    /// What happened to it.
    pub state: ChordState,
}

/// The phase of a pointer drag or a touchpad gesture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DragPhase {
    /// The press (or the gesture's begin).
    Start,
    /// The pointer moved.
    Update,
    /// The button was released.
    End,
    /// The compositor ended it: the session locked, the claim went.
    Cancel,
}

impl DragPhase {
    fn parse(text: &str) -> DragPhase {
        match text {
            "start" | "begin" => DragPhase::Start,
            "end" => DragPhase::End,
            "cancel" | "cancelled" => DragPhase::Cancel,
            _ => DragPhase::Update,
        }
    }
}

/// One step of a pointer chord's drag (`<Super>Button1`): where the pointer is and how far it moved since the press.
#[derive(Clone, Debug, PartialEq)]
pub struct PointerDrag {
    /// Where in the drag this is.
    pub phase: DragPhase,
    /// The pointer's global place.
    pub x: f64,
    /// The pointer's global place.
    pub y: f64,
    /// Horizontal movement since the press.
    pub dx: f64,
    /// Vertical movement since the press.
    pub dy: f64,
    /// The chord's button number (1 left, 2 middle, 3 right, 8 back, 9 forward).
    pub button: i64,
    /// The chord's modifiers as the compositor spells them.
    pub modifiers: String,
    /// The input event's time in milliseconds.
    pub time: i64,
}

/// Which gesture a touchpad claim wants.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GestureKind {
    /// Fingers moving together.
    Swipe,
    /// Fingers moving apart or together.
    Pinch,
}

/// One step of a claimed touchpad gesture.
#[derive(Clone, Debug, PartialEq)]
pub struct GestureEvent {
    /// `swipe` or `pinch`.
    pub kind: String,
    /// How many fingers.
    pub fingers: i64,
    /// Where in the gesture this is.
    pub phase: DragPhase,
    /// Horizontal movement.
    pub dx: f64,
    /// Vertical movement.
    pub dy: f64,
    /// A pinch's scale (1 at the start).
    pub scale: f64,
    /// A pinch's rotation in degrees.
    pub rotation: f64,
}

/// A button of a claimed pointer went down or up.
#[derive(Clone, Debug, PartialEq)]
pub struct PointerButton {
    /// The evdev button code (272 is the left button).
    pub button: i64,
    /// Whether it went down.
    pub pressed: bool,
    /// The pointer's global place.
    pub x: f64,
    /// The pointer's global place.
    pub y: f64,
    /// The input event's time in milliseconds.
    pub time: i64,
}

/// What an input grab takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GrabScope {
    /// Every button, the motion and the wheel of every pointer.
    Pointer,
    /// Every key.
    Keyboard,
    /// Both.
    Both,
}

impl GrabScope {
    fn text(self) -> &'static str {
        match self {
            GrabScope::Pointer => "pointer",
            GrabScope::Keyboard => "keyboard",
            GrabScope::Both => "both",
        }
    }
}

/// What the core says about one event of a claim, borrowed for the callback.
struct RawClaim<'a> {
    kind: u32,
    state: &'a str,
    text: &'a str,
    fingers: i64,
    button: i64,
    x: f64,
    y: f64,
    dx: f64,
    dy: f64,
    scale: f64,
    rotation: f64,
    time: i64,
}

fn str_of<'a>(raw: &GnoblinStr) -> &'a str {
    if raw.ptr.is_null() || raw.len == 0 {
        return "";
    }
    // SAFETY: the core lends `len` bytes at `ptr` until the callback returns, and the callback's `'a` ends there.
    std::str::from_utf8(unsafe { std::slice::from_raw_parts(raw.ptr, raw.len) }).unwrap_or("")
}

unsafe extern "C" fn claim_tramp<F: FnMut(&RawClaim<'_>)>(user: *mut c_void, event: *const GnoblinClaimEvent) -> i32 {
    CALLS.fetch_add(1, Ordering::Relaxed);
    // SAFETY: `user` is the `Box<Slot<F>>` leaked at registration, handed back unchanged on the shell thread and not freed before
    // `drop_slot` runs.
    let slot = unsafe { &mut *user.cast::<Slot<F>>() };
    // SAFETY: the core passes a pointer valid until this callback returns, or null.
    let Some(event) = (unsafe { event.as_ref() }) else {
        return abi::E_ERR;
    };
    let raw = RawClaim {
        kind: event.kind,
        state: str_of(&event.state),
        text: str_of(&event.text),
        fingers: event.fingers,
        button: event.button,
        x: event.x,
        y: event.y,
        dx: event.dx,
        dy: event.dy,
        scale: event.scale,
        rotation: event.rotation,
        time: event.time,
    };
    match catch_unwind(AssertUnwindSafe(|| (slot.f)(&raw))) {
        Ok(()) => abi::E_OK,
        Err(_) => abi::E_PANIC,
    }
}

impl Gnoblin {
    /// Claims `spec` on the seat and calls `f` for each event of the claim.
    fn claim_raw<F>(&self, spec: &GnoblinClaim, f: Option<F>) -> Result<Subscription>
    where
        F: FnMut(&RawClaim<'_>) + 'static,
    {
        self.attach()?;
        let seat = self.main_seat()?;
        match f {
            Some(f) => {
                let slot = Box::into_raw(Box::new(Slot { f, g: self.clone() }));
                // SAFETY: `slot` is a leaked box the core hands back to `claim_tramp::<F>` and `drop_slot::<Slot<F>>`; on failure
                // the core has not adopted it and `adopt` reclaims it. `spec` and the strings it points at live across the call.
                let reg = unsafe {
                    (self.api().claim)(
                        self.ctx(),
                        seat.id().0,
                        spec,
                        Some(claim_tramp::<F>),
                        slot.cast(),
                        Some(drop_slot::<Slot<F>>),
                    )
                };
                self.adopt(reg, slot)
            }
            None => {
                // SAFETY: with no callback there is no `user` to adopt; `spec` and its strings live across the call.
                let reg =
                    unsafe { (self.api().claim)(self.ctx(), seat.id().0, spec, None, core::ptr::null_mut(), None) };
                self.adopt_bare(reg)
            }
        }
    }

    fn claim_spec(kind: u32, flags: u32, fingers: i64, subject: &str, device: &str, node: u64) -> GnoblinClaim {
        GnoblinClaim {
            size: size_of::<GnoblinClaim>() as u32,
            kind,
            flags,
            reserved: 0,
            fingers,
            subject: GnoblinStr::borrowed(subject),
            device: GnoblinStr::borrowed(device),
            node,
        }
    }

    /// Claims the key chord `chord` (`<Super>Return`, `<Ctrl><Alt>Delete`: the compositor's syntax, each modifier in angle
    /// brackets before the key) and calls `f` each time it is pressed. The compositor withholds the chord from every client.
    /// The chord is claimed on the shell's seat, for every keyboard.
    ///
    /// ```ignore
    /// g.bind("<Super>Return", || { g.spawn("foot").ok(); })?;
    /// ```
    pub fn bind<F>(&self, chord: &str, mut f: F) -> Result<Subscription>
    where
        F: FnMut() + 'static,
    {
        self.bind_events(chord, move |event| {
            if event.state == ChordState::Press {
                f();
            }
        })
    }

    /// Claims a key chord and calls `f` for each press, release and repeat of it.
    pub fn bind_events<F>(&self, chord: &str, mut f: F) -> Result<Subscription>
    where
        F: FnMut(&ChordEvent) + 'static,
    {
        let spec = Self::claim_spec(abi::CLAIM_CHORD, abi::CLAIM_FLAG_GRAB, 0, chord, "", 0);
        self.claim_raw(
            &spec,
            Some(move |raw: &RawClaim<'_>| {
                if raw.kind != abi::CLAIM_EVENT_CHORD {
                    return;
                }
                let state = match raw.state {
                    "release" => ChordState::Release,
                    "repeat" => ChordState::Repeat,
                    _ => ChordState::Press,
                };
                f(&ChordEvent {
                    chord: raw.text.to_owned(),
                    state,
                });
            }),
        )
    }

    /// Claims a pointer chord (a chord that ends in `Button1` to `Button9`: `<Super>Button1`) and calls `f` for each step of the
    /// drag. With exactly the chord's modifiers held, the press is withheld from clients.
    pub fn bind_pointer<F>(&self, chord: &str, mut f: F) -> Result<Subscription>
    where
        F: FnMut(&PointerDrag) + 'static,
    {
        let spec = Self::claim_spec(abi::CLAIM_CHORD, 0, 0, chord, "", 0);
        self.claim_raw(
            &spec,
            Some(move |raw: &RawClaim<'_>| {
                if raw.kind != abi::CLAIM_EVENT_POINTER_CHORD {
                    return;
                }
                f(&PointerDrag {
                    phase: DragPhase::parse(raw.state),
                    x: raw.x,
                    y: raw.y,
                    dx: raw.dx,
                    dy: raw.dy,
                    button: raw.button,
                    modifiers: raw.text.to_owned(),
                    time: raw.time,
                });
            }),
        )
    }

    /// Claims every touchpad gesture of `kind` with `fingers` contacts (2 to 5) and calls `f` for each step.
    pub fn bind_gesture<F>(&self, kind: GestureKind, fingers: u8, mut f: F) -> Result<Subscription>
    where
        F: FnMut(&GestureEvent) + 'static,
    {
        let name = match kind {
            GestureKind::Swipe => "swipe",
            GestureKind::Pinch => "pinch",
        };
        let spec = Self::claim_spec(abi::CLAIM_GESTURE, 0, i64::from(fingers), name, "", 0);
        self.claim_raw(
            &spec,
            Some(move |raw: &RawClaim<'_>| {
                if raw.kind != abi::CLAIM_EVENT_GESTURE {
                    return;
                }
                f(&GestureEvent {
                    kind: raw.text.to_owned(),
                    fingers: raw.fingers,
                    phase: DragPhase::parse(raw.state),
                    dx: raw.dx,
                    dy: raw.dy,
                    scale: raw.scale,
                    rotation: raw.rotation,
                });
            }),
        )
    }

    /// Claims every pointer button of the pointers that report `device` (a name from the seat's `device`) and calls `f` for each
    /// press and release: the buttons reach no client or node while the claim stands.
    pub fn claim_pointer_buttons<F>(&self, device: &str, mut f: F) -> Result<Subscription>
    where
        F: FnMut(&PointerButton) + 'static,
    {
        let spec = Self::claim_spec(abi::CLAIM_POINTER, 0, 0, "", device, 0);
        self.claim_raw(
            &spec,
            Some(move |raw: &RawClaim<'_>| {
                if raw.kind != abi::CLAIM_EVENT_POINTER_BUTTON {
                    return;
                }
                f(&PointerButton {
                    button: raw.button,
                    pressed: raw.state == "press",
                    x: raw.x,
                    y: raw.y,
                    time: raw.time,
                });
            }),
        )
    }

    /// Makes the devices that report `device` binding-only: their keys reach no client and fire only the chords claimed for them or
    /// for every device, while the claim stands.
    pub fn claim_device(&self, device: &str) -> Result<Subscription> {
        let spec = Self::claim_spec(abi::CLAIM_DEVICE, 0, 0, "", device, 0);
        self.claim_raw::<fn(&RawClaim<'_>)>(&spec, None)
    }

    /// Takes the seat's pointer, keyboard or both for `node`, a node of the shell's own: while the grab stands, no client, claim or
    /// other node hears what it takes, and `node` hears it as events ([`Gnoblin::on_node_event`]). One grab at a time.
    pub fn grab_input(&self, node: &Node, scope: GrabScope) -> Result<Subscription> {
        let spec = Self::claim_spec(abi::CLAIM_GRAB, 0, 0, scope.text(), "", node.id().0);
        self.claim_raw::<fn(&RawClaim<'_>)>(&spec, None)
    }
}
