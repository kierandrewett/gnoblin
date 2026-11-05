//! One test per documented failure mode, in the order `eis`'s crate docs list them.
//!
//! These drive the state machine directly: no socket, no client, no clock. What they
//! prove is the rule — what a client may send and what happens to everything it holds
//! when the answer is no. The same rules are then exercised over a real socket by
//! `tests/eis_real.rs`.

use gnoblin_backend::{InputEvent, TouchPhase};
use gnoblin_input::eis::{
    Action, Caps, Command, Context, ContextId, ContextType, Device, DeviceId, Ending, Grant, Origin, Quota, Region,
    Service, Violation,
};
use gnoblin_types::{OutputId, Point, Rect};

/// The output a test's layout starts with.
const OUTPUT: OutputId = OutputId::new(1);
/// A second output, to the right of the first.
const SECOND: OutputId = OutputId::new(2);
/// `KEY_A` from `linux/input-event-codes.h`.
const KEY_A: u32 = 30;
/// `KEY_B`.
const KEY_B: u32 = 48;
/// `BTN_LEFT`.
const BTN_LEFT: u32 = 0x110;

/// The layout the two regions describe: 1920x1080 at the origin, then 1280x1024 to its
/// right.
fn layout() -> Vec<Region> {
    vec![
        Region {
            output: OUTPUT,
            rect: Rect::new(0, 0, 1920, 1080),
        },
        Region {
            output: SECOND,
            rect: Rect::new(1920, 0, 1280, 1024),
        },
    ]
}

/// Everything the state machine can grant.
fn all_caps() -> Caps {
    Caps::POINTER | Caps::POINTER_ABSOLUTE | Caps::KEYBOARD | Caps::BUTTON | Caps::SCROLL | Caps::TOUCH
}

/// A service, one connected client, and the three devices a full bind creates.
struct Peer {
    service: Service,
    id: ContextId,
    pointer: Option<DeviceId>,
    keyboard: Option<DeviceId>,
    touch: Option<DeviceId>,
}

impl Peer {
    /// A client that has connected with this decision and done nothing else.
    fn connect(grant: Grant) -> Self {
        let mut service = Service::new(grant, layout(), Quota::default());
        let (id, _) = service.connect(Some("test-client"), ContextType::Sender);
        Self {
            service,
            id,
            pointer: None,
            keyboard: None,
            touch: None,
        }
    }

    /// A client with everything granted, the seat bound and every device emulating.
    fn ready() -> Self {
        let mut peer = Self::connect(Grant::granted(all_caps()));
        peer.bind(all_caps());
        peer.emulate();
        peer
    }

    /// A client with everything granted and the seat bound, but no device emulating.
    fn bound() -> Self {
        let mut peer = Self::connect(Grant::granted(all_caps()));
        peer.bind(all_caps());
        peer
    }

    fn apply(&mut self, command: Command) -> Vec<Action> {
        self.service.apply(self.id, command)
    }

    fn bind(&mut self, capabilities: Caps) -> Vec<Action> {
        let actions = self.apply(Command::Bind { capabilities });
        for action in &actions {
            if let Action::Device {
                device, capabilities, ..
            } = action
            {
                if capabilities.contains(Caps::KEYBOARD) {
                    self.keyboard = Some(*device);
                } else if capabilities.contains(Caps::TOUCH) {
                    self.touch = Some(*device);
                } else {
                    self.pointer = Some(*device);
                }
            }
        }
        actions
    }

    fn emulate(&mut self) {
        for device in [self.pointer, self.keyboard, self.touch].into_iter().flatten() {
            self.apply(Command::StartEmulating { device, sequence: 1 });
        }
    }

    fn context(&self) -> &Context {
        self.service.context(self.id).expect("the context exists")
    }

    fn device(&self, id: Option<DeviceId>) -> &Device {
        self.context()
            .device(id.expect("the device was bound"))
            .expect("the device exists")
    }

    fn keyboard_device(&self) -> DeviceId {
        self.keyboard.expect("the keyboard was bound")
    }

    fn pointer_device(&self) -> DeviceId {
        self.pointer.expect("the pointer was bound")
    }

    fn touch_device(&self) -> DeviceId {
        self.touch.expect("the touch device was bound")
    }

    fn key(&mut self, key: u32, pressed: bool) -> Vec<Action> {
        let device = self.keyboard_device();
        self.apply(Command::Key { device, key, pressed })
    }

    fn button(&mut self, button: u32, pressed: bool) -> Vec<Action> {
        let device = self.pointer_device();
        self.apply(Command::Button {
            device,
            button,
            pressed,
        })
    }

    fn frame(&mut self, device: DeviceId) -> Vec<Action> {
        self.apply(Command::Frame { device })
    }
}

/// The events an action list delivers to the seat, in order.
fn injections(actions: &[Action]) -> Vec<InputEvent> {
    actions
        .iter()
        .filter_map(|action| match action {
            Action::Inject(injection) => Some(injection.event()),
            _ => None,
        })
        .collect()
}

/// The ending an action list carries, if it ends the connection.
fn ending(actions: &[Action]) -> Option<Ending> {
    actions.iter().find_map(|action| match action {
        Action::End { ending } => Some(*ending),
        _ => None,
    })
}

/// The devices an action list creates.
fn devices(actions: &[Action]) -> Vec<Caps> {
    actions
        .iter()
        .filter_map(|action| match action {
            Action::Device { capabilities, .. } => Some(*capabilities),
            _ => None,
        })
        .collect()
}

#[test]
fn a_disconnect_releases_every_held_edge_once() {
    let mut peer = Peer::ready();
    peer.key(KEY_A, true);
    peer.button(BTN_LEFT, true);

    let actions = peer.apply(Command::Disconnect);
    let released = injections(&actions);

    assert_eq!(released.len(), 2, "one release per held edge: {released:?}");
    assert!(released.contains(&InputEvent::Key {
        keycode: KEY_A,
        pressed: false
    }));
    assert!(released.contains(&InputEvent::PointerButton {
        button: BTN_LEFT,
        pressed: false
    }));
    assert_eq!(ending(&actions), Some(Ending::Disconnected));

    // A second disconnect has nothing left to release and does not repeat the ending.
    assert_eq!(peer.apply(Command::Disconnect), Vec::new());
}

#[test]
fn a_stop_emulating_releases_what_the_device_holds() {
    let mut peer = Peer::ready();
    let keyboard = peer.keyboard_device();
    peer.key(KEY_A, true);

    let actions = peer.apply(Command::StopEmulating { device: keyboard });
    assert_eq!(
        injections(&actions),
        vec![InputEvent::Key {
            keycode: KEY_A,
            pressed: false
        }]
    );
    assert_eq!(peer.device(peer.keyboard).held_keys().count(), 0);
    // And the device may not send again until it emulates once more.
    let actions = peer.key(KEY_A, true);
    assert_eq!(ending(&actions), Some(Ending::Violation(Violation::NotEmulating)));
}

#[test]
fn a_bind_beyond_the_grant_creates_no_device() {
    let mut peer = Peer::connect(Grant::granted(Caps::KEYBOARD));
    assert_eq!(
        peer.context().capabilities(),
        Caps::KEYBOARD,
        "the seat advertises only what the grant covers"
    );

    let actions = peer.bind(Caps::KEYBOARD | Caps::POINTER);
    assert_eq!(
        devices(&actions),
        Vec::new(),
        "no device may come from an unadvertised capability"
    );
    assert_eq!(
        ending(&actions),
        Some(Ending::Violation(Violation::CapabilityEscalation))
    );
}

#[test]
fn a_granted_capability_alone_creates_its_device() {
    let mut peer = Peer::connect(Grant::granted(Caps::KEYBOARD));
    let actions = peer.bind(Caps::KEYBOARD);
    assert_eq!(devices(&actions), vec![Caps::KEYBOARD]);
}

#[test]
fn an_absolute_event_outside_every_region_is_refused() {
    let mut peer = Peer::ready();
    let device = peer.pointer_device();

    let actions = peer.apply(Command::AbsoluteMotion {
        device,
        x: 4000.0,
        y: 10.0,
    });
    assert_eq!(ending(&actions), Some(Ending::Violation(Violation::OutsideRegions)));

    // A position that is not a number cannot be placed anywhere, and is refused rather
    // than rounded onto an output.
    let mut peer = Peer::ready();
    let device = peer.pointer_device();
    let actions = peer.apply(Command::AbsoluteMotion {
        device,
        x: f32::NAN,
        y: 10.0,
    });
    assert_eq!(ending(&actions), Some(Ending::Violation(Violation::Malformed)));
}

#[test]
fn an_absolute_event_without_a_region_is_refused() {
    // A layout with no outputs means a device with no region, which the protocol calls an
    // implementation bug for an absolute capability; the event is refused rather than
    // clamped to an output that does not exist.
    let mut service = Service::new(Grant::granted(Caps::POINTER_ABSOLUTE), Vec::new(), Quota::default());
    let (id, _) = service.connect(Some("no-outputs"), ContextType::Sender);
    let actions = service.apply(
        id,
        Command::Bind {
            capabilities: Caps::POINTER_ABSOLUTE,
        },
    );
    let device = match &actions[0] {
        Action::Device { device, .. } => *device,
        other => panic!("a device was expected, got {other:?}"),
    };
    service.apply(id, Command::StartEmulating { device, sequence: 1 });
    let actions = service.apply(id, Command::AbsoluteMotion { device, x: 1.0, y: 1.0 });
    assert_eq!(ending(&actions), Some(Ending::Violation(Violation::NoRegions)));
}

#[test]
fn an_absolute_event_lands_in_the_region_it_names() {
    let mut peer = Peer::ready();
    let device = peer.pointer_device();

    let actions = peer.apply(Command::AbsoluteMotion {
        device,
        x: 2000.0,
        y: 40.0,
    });
    assert_eq!(
        injections(&actions),
        vec![InputEvent::PointerMotion {
            output: SECOND,
            position: Point::new(80, 40)
        }],
        "the position is local to the output the region belongs to"
    );
}

#[test]
fn a_flood_ends_the_session() {
    let mut service = Service::new(
        Grant::granted(Caps::KEYBOARD),
        layout(),
        Quota {
            events_per_frame: 3,
            contacts: 8,
        },
    );
    let (id, _) = service.connect(Some("flood"), ContextType::Sender);
    let actions = service.apply(
        id,
        Command::Bind {
            capabilities: Caps::KEYBOARD,
        },
    );
    let device = match &actions[0] {
        Action::Device { device, .. } => *device,
        other => panic!("a device was expected, got {other:?}"),
    };
    service.apply(id, Command::StartEmulating { device, sequence: 1 });

    let mut last = Vec::new();
    for offset in 0..4u32 {
        last = service.apply(
            id,
            Command::Key {
                device,
                key: KEY_A + offset,
                pressed: true,
            },
        );
        if ending(&last).is_some() {
            break;
        }
    }
    assert_eq!(ending(&last), Some(Ending::Violation(Violation::Flood)));
    let released = injections(&last);
    assert_eq!(released.len(), 3, "the three keys already held are released first");
}

#[test]
fn a_device_event_before_start_emulating_is_refused() {
    let mut peer = Peer::bound();
    let actions = peer.key(KEY_A, true);
    assert_eq!(ending(&actions), Some(Ending::Violation(Violation::NotEmulating)));
    assert!(
        injections(&actions).is_empty(),
        "a refused event never reaches the seat"
    );
}

#[test]
fn a_second_start_emulating_is_refused() {
    let mut peer = Peer::ready();
    let device = peer.keyboard_device();
    let actions = peer.apply(Command::StartEmulating { device, sequence: 2 });
    assert_eq!(ending(&actions), Some(Ending::Violation(Violation::AlreadyEmulating)));
}

#[test]
fn a_sequence_that_does_not_increase_is_refused() {
    let mut peer = Peer::ready();
    let device = peer.keyboard_device();
    peer.apply(Command::StopEmulating { device });
    let actions = peer.apply(Command::StartEmulating { device, sequence: 1 });
    assert_eq!(
        ending(&actions),
        Some(Ending::Violation(Violation::SequenceNotIncreasing))
    );
}

#[test]
fn an_event_for_an_unknown_device_is_refused() {
    let mut peer = Peer::ready();
    let device = peer.pointer_device();
    peer.apply(Command::Release { device });

    let actions = peer.apply(Command::AbsoluteMotion {
        device,
        x: 10.0,
        y: 10.0,
    });
    assert_eq!(ending(&actions), Some(Ending::Violation(Violation::UnknownDevice)));
}

#[test]
fn a_released_device_is_destroyed_after_its_edges() {
    let mut peer = Peer::ready();
    let device = peer.keyboard_device();
    peer.key(KEY_A, true);

    let actions = peer.apply(Command::Release { device });
    assert_eq!(
        injections(&actions),
        vec![InputEvent::Key {
            keycode: KEY_A,
            pressed: false
        }]
    );
    assert!(matches!(actions.last(), Some(Action::DestroyDevice { device: destroyed }) if *destroyed == device));
    assert!(peer.context().device(device).is_none());
}

#[test]
fn an_event_for_a_capability_the_device_lacks_is_refused() {
    let mut peer = Peer::ready();
    let keyboard = peer.keyboard_device();
    let actions = peer.apply(Command::AbsoluteMotion {
        device: keyboard,
        x: 10.0,
        y: 10.0,
    });
    assert_eq!(ending(&actions), Some(Ending::Violation(Violation::WrongCapability)));
}

#[test]
fn a_key_code_outside_the_linux_range_is_refused() {
    let mut peer = Peer::ready();
    let actions = peer.key(0x300, true);
    assert_eq!(ending(&actions), Some(Ending::Violation(Violation::KeyOutOfRange)));

    let mut peer = Peer::ready();
    let actions = peer.button(0x300, true);
    assert_eq!(ending(&actions), Some(Ending::Violation(Violation::ButtonOutOfRange)));
}

#[test]
fn a_duplicate_edge_in_one_frame_is_refused() {
    let mut peer = Peer::ready();
    peer.key(KEY_A, true);
    let actions = peer.key(KEY_A, false);
    assert_eq!(ending(&actions), Some(Ending::Violation(Violation::RepeatedInFrame)));

    // Two different keys in one frame are two edges, and both are delivered.
    let mut peer = Peer::ready();
    peer.key(KEY_A, true);
    let actions = peer.key(KEY_B, true);
    assert!(ending(&actions).is_none());
    assert_eq!(injections(&actions).len(), 1);
}

#[test]
fn a_press_of_an_already_held_edge_is_refused() {
    let mut peer = Peer::ready();
    let device = peer.keyboard_device();
    peer.key(KEY_A, true);
    peer.frame(device);
    let actions = peer.key(KEY_A, true);
    assert_eq!(ending(&actions), Some(Ending::Violation(Violation::AlreadyHeld)));
}

#[test]
fn an_orphaned_release_is_dropped() {
    let mut peer = Peer::ready();
    let actions = peer.key(KEY_A, false);
    assert!(ending(&actions).is_none(), "an orphan release is not a protocol error");
    assert!(injections(&actions).is_empty(), "nothing the user pressed is released");
    assert_eq!(peer.context().dropped_releases(), 1);
}

#[test]
fn a_touch_contact_must_be_down_before_it_moves() {
    let mut peer = Peer::ready();
    let device = peer.touch_device();
    let actions = peer.apply(Command::TouchMotion {
        device,
        contact: 7,
        x: 10.0,
        y: 10.0,
    });
    assert_eq!(ending(&actions), Some(Ending::Violation(Violation::NoSuchContact)));
}

#[test]
fn a_touch_down_and_up_reach_the_seat_once_each() {
    let mut peer = Peer::ready();
    let device = peer.touch_device();
    let actions = peer.apply(Command::TouchDown {
        device,
        contact: 3,
        x: 1900.0,
        y: 20.0,
    });
    assert_eq!(
        injections(&actions),
        vec![InputEvent::Touch {
            output: OUTPUT,
            slot: 3,
            position: Point::new(1900, 20),
            phase: TouchPhase::Down
        }]
    );
    let actions = peer.apply(Command::TouchUp { device, contact: 3 });
    assert_eq!(
        injections(&actions),
        vec![InputEvent::Touch {
            output: OUTPUT,
            slot: 3,
            position: Point::new(1900, 20),
            phase: TouchPhase::Up
        }]
    );
}

#[test]
fn a_disconnect_cancels_the_contacts_it_holds() {
    let mut peer = Peer::ready();
    let device = peer.touch_device();
    peer.apply(Command::TouchDown {
        device,
        contact: 1,
        x: 5.0,
        y: 5.0,
    });
    let actions = peer.apply(Command::Disconnect);
    assert_eq!(
        injections(&actions),
        vec![InputEvent::Touch {
            output: OUTPUT,
            slot: 1,
            position: Point::new(5, 5),
            phase: TouchPhase::Cancel
        }]
    );
}

#[test]
fn more_contacts_than_the_quota_is_refused() {
    let mut service = Service::new(
        Grant::granted(Caps::TOUCH),
        layout(),
        Quota {
            events_per_frame: 32,
            contacts: 2,
        },
    );
    let (id, _) = service.connect(Some("many-fingers"), ContextType::Sender);
    let actions = service.apply(
        id,
        Command::Bind {
            capabilities: Caps::TOUCH,
        },
    );
    let device = match &actions[0] {
        Action::Device { device, .. } => *device,
        other => panic!("a device was expected, got {other:?}"),
    };
    service.apply(id, Command::StartEmulating { device, sequence: 1 });
    service.apply(
        id,
        Command::TouchDown {
            device,
            contact: 1,
            x: 1.0,
            y: 1.0,
        },
    );
    service.apply(
        id,
        Command::TouchDown {
            device,
            contact: 2,
            x: 2.0,
            y: 2.0,
        },
    );
    let actions = service.apply(
        id,
        Command::TouchDown {
            device,
            contact: 3,
            x: 3.0,
            y: 3.0,
        },
    );
    assert_eq!(ending(&actions), Some(Ending::Violation(Violation::TooManyContacts)));
}

#[test]
fn a_refusal_ends_the_connection_with_its_reason() {
    let mut service = Service::new(Grant::pending(), layout(), Quota::default());
    let (id, actions) = service.connect(Some("waiting"), ContextType::Sender);
    assert_eq!(
        actions,
        vec![Action::Seat {
            capabilities: Caps::NONE
        }],
        "a pending decision advertises nothing"
    );

    let told = service.set_grant(Grant::refused());
    assert_eq!(told.len(), 1);
    assert_eq!(told[0].0, id);
    assert_eq!(ending(std::slice::from_ref(&told[0].1)), Some(Ending::Refused));

    // A client that connects afterwards is refused at once, and its seat is never made.
    let (_, actions) = service.connect(Some("late"), ContextType::Sender);
    assert_eq!(ending(&actions), Some(Ending::Refused));
}

#[test]
fn a_pending_grant_advertises_nothing_until_the_decision() {
    let mut service = Service::new(Grant::pending(), layout(), Quota::default());
    let (id, _) = service.connect(Some("waiting"), ContextType::Sender);

    // Nothing was advertised, so a bind for anything is an escalation.
    let actions = service.apply(
        id,
        Command::Bind {
            capabilities: Caps::KEYBOARD,
        },
    );
    assert_eq!(
        ending(&actions),
        Some(Ending::Violation(Violation::CapabilityEscalation))
    );

    // Now the decision arrives.
    let mut service = Service::new(Grant::pending(), layout(), Quota::default());
    let (id, _) = service.connect(Some("waiting"), ContextType::Sender);
    let told = service.set_grant(Grant::granted(Caps::KEYBOARD));
    assert_eq!(
        told,
        vec![(
            id,
            Action::Seat {
                capabilities: Caps::KEYBOARD
            }
        )]
    );
    let actions = service.apply(
        id,
        Command::Bind {
            capabilities: Caps::KEYBOARD,
        },
    );
    assert_eq!(devices(&actions), vec![Caps::KEYBOARD]);
}

#[test]
fn a_receiver_context_is_refused() {
    let mut service = Service::new(Grant::granted(all_caps()), layout(), Quota::default());
    let (_, actions) = service.connect(Some("receiver"), ContextType::Receiver);
    assert_eq!(
        ending(&actions),
        Some(Ending::Violation(Violation::UnsupportedContext)),
        "this server injects; it does not deliver input to a client"
    );
}

#[test]
fn a_layout_change_does_not_move_an_existing_devices_regions() {
    let mut peer = Peer::ready();
    let device = peer.pointer_device();
    let regions = peer.device(peer.pointer).regions().to_vec();
    assert_eq!(regions.len(), 2);

    peer.service.set_regions(Vec::new());
    assert_eq!(
        peer.device(peer.pointer).regions(),
        regions.as_slice(),
        "the layout a device was created with is the one it keeps"
    );
    let actions = peer.apply(Command::AbsoluteMotion {
        device,
        x: 10.0,
        y: 10.0,
    });
    assert!(
        ending(&actions).is_none(),
        "the old layout still resolves positions: {actions:?}"
    );
}

#[test]
fn an_absolute_event_uses_the_translated_layout() {
    // A monitor left of the primary output gives the layout a negative origin, which
    // `ei_device.region` cannot state. The device's regions are translated so the
    // top-left of the whole layout is the origin, and an event in that space still lands
    // on the right output at the right local position.
    let left = Region {
        output: SECOND,
        rect: Rect::new(-1280, 0, 1280, 1024),
    };
    let right = Region {
        output: OUTPUT,
        rect: Rect::new(0, 0, 1920, 1080),
    };
    let mut service = Service::new(
        Grant::granted(Caps::POINTER_ABSOLUTE),
        vec![left, right],
        Quota::default(),
    );
    let (id, _) = service.connect(Some("negative"), ContextType::Sender);
    let actions = service.apply(
        id,
        Command::Bind {
            capabilities: Caps::POINTER_ABSOLUTE,
        },
    );
    let Action::Device { device, regions, .. } = &actions[0] else {
        panic!("a device was expected, got {:?}", actions[0]);
    };
    assert_eq!(
        regions,
        &vec![
            Region {
                output: SECOND,
                rect: Rect::new(0, 0, 1280, 1024)
            },
            Region {
                output: OUTPUT,
                rect: Rect::new(1280, 0, 1920, 1080)
            }
        ]
    );
    let device = *device;
    service.apply(id, Command::StartEmulating { device, sequence: 1 });
    let actions = service.apply(
        id,
        Command::AbsoluteMotion {
            device,
            x: 1300.0,
            y: 10.0,
        },
    );
    assert_eq!(
        injections(&actions),
        vec![InputEvent::PointerMotion {
            output: OUTPUT,
            position: Point::new(20, 10)
        }]
    );
}

#[test]
fn an_injected_event_carries_the_eis_origin() {
    let mut peer = Peer::ready();
    let actions = peer.key(KEY_A, true);
    let Action::Inject(injection) = actions[0] else {
        panic!("an injection was expected, got {:?}", actions[0]);
    };
    assert_eq!(injection.origin(), Origin::Eis);
    assert_eq!(
        injection.into_event(),
        InputEvent::Key {
            keycode: KEY_A,
            pressed: true
        }
    );
}

#[test]
fn a_relative_motion_reaches_the_seat_as_a_delta() {
    let mut peer = Peer::ready();
    let device = peer.pointer_device();
    let actions = peer.apply(Command::RelativeMotion {
        device,
        dx: 4.6,
        dy: -3.2,
    });
    assert_eq!(
        injections(&actions),
        vec![InputEvent::PointerMotionRelative { dx: 5, dy: -3 }]
    );
}

#[test]
fn a_discrete_scroll_reaches_the_seat_in_wheel_units() {
    let mut peer = Peer::ready();
    let device = peer.pointer_device();
    let actions = peer.apply(Command::ScrollDiscrete {
        device,
        dx: 0,
        dy: -120,
    });
    assert_eq!(
        injections(&actions),
        vec![InputEvent::PointerAxis {
            horizontal: 0,
            vertical: -120
        }]
    );
    // A smooth delta is converted at the same rate the nested backend uses: ten logical
    // pixels to one notch, and one notch is 120 units.
    let actions = peer.apply(Command::ScrollDelta {
        device,
        dx: 5.0,
        dy: 10.0,
    });
    assert_eq!(
        injections(&actions),
        vec![InputEvent::PointerAxis {
            horizontal: 60,
            vertical: 120
        }]
    );
}

#[test]
fn the_same_commands_produce_the_same_actions() {
    let script = |peer: &mut Peer| {
        let keyboard = peer.keyboard_device();
        let pointer = peer.pointer_device();
        let mut actions = Vec::new();
        actions.extend(peer.key(KEY_A, true));
        actions.extend(peer.key(KEY_B, true));
        actions.extend(peer.frame(keyboard));
        actions.extend(peer.button(BTN_LEFT, true));
        actions.extend(peer.apply(Command::AbsoluteMotion {
            device: pointer,
            x: 100.0,
            y: 200.0,
        }));
        actions.extend(peer.apply(Command::Disconnect));
        actions
    };
    let mut first = Peer::ready();
    let mut second = Peer::ready();
    let first = script(&mut first);
    let second = script(&mut second);
    assert_eq!(first, second);
}

#[test]
fn a_forgotten_context_stops_being_reported() {
    let mut peer = Peer::ready();
    peer.apply(Command::Disconnect);
    assert_eq!(
        peer.service.contexts().count(),
        1,
        "an ended context is still reportable"
    );
    assert!(peer.service.forget(peer.id).is_some());
    assert_eq!(peer.service.contexts().count(), 0);
    assert!(peer.service.apply(peer.id, Command::Disconnect).is_empty());
}
