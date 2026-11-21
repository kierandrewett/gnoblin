// Example 32 in TypeScript: the window policy of libgnoblin-std, through the classes generated from its catalog.
//
//     gnoblin ./shell.ts
//
// Starts the focus policy, the stock answers to the requests of windows, the keyboard move and resize, the move and resize drags and the
// three-finger workspace slide. A listener of this shell hears every request of a window first and logs it; it refuses `fullscreen` itself, so
// the stock answer does not run. Every line starts with `policy:`, as the Rust shell of the same example spells it.
import {
    FocusPolicy,
    MoveBinding,
    Repositioning,
    RequestListener,
    ResizeBinding,
    WindowState,
    WorkspaceSlideBinding,
} from "gnoblin-std/generated/objects";
import type { ClientRequest } from "gnoblin-std/generated/objects";
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

console.log("policy: running");
