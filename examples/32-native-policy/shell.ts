// Example 32 in TypeScript: the window policy of libgnoblin-std, through the classes generated from its catalog.
//
//     gnoblin ./shell.ts
//
// Starts the focus policy, the stock answers to the requests of windows, the keyboard move and resize, the move and resize drags and the
// three-finger workspace slide. A listener of this shell hears every request of a window first and logs it; it refuses `fullscreen` itself, so
// the stock answer does not run. Every line starts with `policy:`, as the Rust shell of the same example spells it.
import {
    FocusPolicy,
    Gestures,
    MoveBinding,
    PinchActionBinding,
    Repositioning,
    RequestListener,
    ResizeBinding,
    SwipeActionBinding,
    WindowState,
    WorkspaceSlideBinding,
} from "gnoblin-std";
import type { ClientRequest } from "gnoblin-std";
import type {} from "gnoblin-std";

FocusPolicy.get().start();

class Listener extends RequestListener {
    onRequest(request: ClientRequest): void {
        console.log(`policy: request ${request.kind} of window ${request.client}`);
        if (request.kind === "fullscreen") {
            request.handle();
            console.log("policy: fullscreen refused");
        }
    }
}

const state = WindowState.get();
state.addListener(new Listener());
state.start();

Repositioning.get().start();

new MoveBinding().install();
new ResizeBinding().install();
new WorkspaceSlideBinding().install();

// The gestures that run actions: a four-finger swipe up maximises (or restores) the window with the keyboard, a swipe left runs a function, and
// a pinch in minimises. The ways and the actions are checked when they are set.
const swipe = new SwipeActionBinding();
swipe.trigger = "Swipe4";
swipe.setAction("up", "window.toggle-maximise");
swipe.setFunction("left", (event) => console.log(`policy: swipe left went ${event.dx}`));
swipe.install();
const pinch = new PinchActionBinding();
pinch.setAction("in", "window.minimise");
pinch.setFunction("out", (event) => console.log(`policy: pinch out to ${event.scale}`));
pinch.install();
console.log("policy: the gesture bindings are installed");

// The way a gesture has gone, by its longer axis, and none while it has gone less than the threshold.
console.log(`policy: gesture direction ${Gestures.direction(-100, 5)?.way ?? "none"}`);
console.log(`policy: gesture direction ${Gestures.direction(1, 1)?.way ?? "none"}`);
console.log(`policy: gesture direction ${Gestures.direction(1, 1, 0.5)?.way ?? "none"}`);
try {
    swipe.setAction("down", "window.fly");
} catch {
    console.log("policy: an action that is not one is refused");
}

console.log("policy: running");
