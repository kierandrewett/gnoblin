//! Example 32: the window policy of `libgnoblin-std`.
//!
//! The policy is what the desktop does with focus, with the requests a window makes of the shell, and with the pointer and the touchpad. Each
//! part is a service that does nothing before `start`, so a shell with a policy of its own starts none of them. This shell starts:
//!
//! - `FocusPolicy`: a click focuses and raises a window, and a new window takes focus (the seat's `focus_mode` and its other keys decide the
//!   rest);
//! - `WindowState`: the stock answers to a window's request to maximise, minimise, go fullscreen, close or move. A listener of this shell hears
//!   each request first, logs it, and answers `fullscreen` itself by refusing it: the stock answer does not run for a request a listener took;
//! - `Repositioning`: the window menu's Move and Resize move a window with the arrow keys;
//! - the drags and the gesture: `Super` and the left button move a window, `Super` and the right button resize it, and a three-finger swipe
//!   slides to the next or the previous workspace;
//! - the gestures that run actions: a four-finger swipe up toggles maximise and a swipe left runs a function, a pinch in minimises and a pinch
//!   out runs a function. `Gestures::direction` says which way a gesture has gone.
//!
//! Every line it logs starts with `policy:`. Build it with `cargo build -p gnoblin-example-native-policy` and run it with
//! `gnoblin ./target/debug/libnative_policy.so`.

use gnoblin_std::prelude::*;
use gnoblin_std::{
    ClientRequestKind, FocusPolicy, Gestures, MoveBinding, PinchActionBinding, PinchWay, Repositioning,
    RequestListenerHandlers, ResizeBinding, SwipeActionBinding, SwipeWay, WindowState, WorkspaceSlideBinding,
};

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin_std::Result<()> {
    let objects = g.objects()?.std()?;

    FocusPolicy::get(&objects)?.start()?;

    // A listener hears every request of a window before the stock answer, and the first that takes one answers it.
    let state = WindowState::get(&objects)?;
    let listener = RequestListenerHandlers {
        on_request: Box::new(move |request| {
            let kind = request.kind()?;
            g.log(&format!(
                "policy: request {} of window {}",
                kind.as_str(),
                request.client()?
            ));
            if kind == ClientRequestKind::Fullscreen {
                request.take()?;
                g.log("policy: fullscreen refused");
            }
            Ok(())
        }),
    }
    .implement(&objects)?;
    state.add_listener(&listener, None)?;
    // The service holds the listener from here on.
    std::mem::forget(listener);
    state.start()?;

    Repositioning::get(&objects)?.start()?;

    let moving = MoveBinding::new(&objects)?;
    moving.install(None)?;
    std::mem::forget(moving);
    let resizing = ResizeBinding::new(&objects)?;
    resizing.install(None)?;
    std::mem::forget(resizing);
    let slide = WorkspaceSlideBinding::new(&objects)?;
    slide.install(None)?;
    std::mem::forget(slide);

    // The gestures that run actions. The ways and the actions are checked when they are set.
    let swipe = SwipeActionBinding::new(&objects)?;
    swipe.set_trigger("Swipe4")?;
    swipe.set_action(SwipeWay::Up, "window.toggle-maximise")?;
    swipe.set_function(SwipeWay::Left, move |event| {
        g.log(&format!("policy: swipe left went {}", event.dx));
        Ok(())
    })?;
    swipe.install(None)?;
    let pinch = PinchActionBinding::new(&objects)?;
    pinch.set_action(PinchWay::In, "window.minimise")?;
    pinch.set_function(PinchWay::Out, move |event| {
        g.log(&format!("policy: pinch out to {}", event.scale));
        Ok(())
    })?;
    pinch.install(None)?;
    g.log("policy: the gesture bindings are installed");

    // The way a gesture has gone, by its longer axis, and none while it has gone less than the threshold.
    for (dx, dy, threshold) in [(-100.0, 5.0, None), (1.0, 1.0, None), (1.0, 1.0, Some(0.5))] {
        let way = Gestures::direction(&objects, dx, dy, threshold)?;
        g.log(&format!(
            "policy: gesture direction {}",
            way.map_or("none", |direction| direction.way.as_str())
        ));
    }
    if swipe.set_action(SwipeWay::Down, "window.fly").is_err() {
        g.log("policy: an action that is not one is refused");
    }
    std::mem::forget(swipe);
    std::mem::forget(pinch);

    g.log("policy: running");
    Ok(())
}
