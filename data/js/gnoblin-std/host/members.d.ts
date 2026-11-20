import type { FrameInset } from "gnoblin-core/inset";
import { Client } from "gnoblin-core/host/client";
import type { Member as HostMember } from "gnoblin-core/host/collection_state";
import type { MutableCollection } from "gnoblin-core/host/collections";
import { Output } from "gnoblin-core/host/output";
import type { Rect } from "gnoblin-core/types";
import type { Display } from "gnoblin-std/display/display";
import type { OutputProfile } from "gnoblin-std/display/output_profile";
import { Extensions } from "gnoblin-std/extensions/xr/availability";
import type { DisplayCollection, WorkspaceCollection } from "gnoblin-std/host/roots";
import type { ShowOptions } from "gnoblin-std/host/show";
import type { Workspace } from "gnoblin-std/host/workspace";
import type { Scene } from "gnoblin-std/scene/scene";
import type { ComputedStyle } from "gnoblin-std/style/computed";
import type { ClientStyle, ClientStyleInit } from "gnoblin-std/style/style";
import type { Transition } from "gnoblin-std/transition/transition";
import type { TransitionOptions } from "gnoblin-std/transition/types";
import type { GestureTools } from "gnoblin-std/gesture_actions";
import type { ClientRequest } from "gnoblin-std/window_state";
declare module "gnoblin-core/host/collection_state" {
    interface MemberKinds {
        Workspace: Workspace;
        Display: Display;
        OutputProfile: OutputProfile;
    }
}
declare module "gnoblin-core/host/client" {
    interface Client {
        /**
         * The overrides this client has, and only those: a property that was never set here is `undefined` and comes from
         * the scopes above (`gnoblin.clients.style`, the workspace's, a `where()` selection's). Reading a list this
         * client does not override gives a copy of the inherited one; the first write to the copy makes it this client's
         * own.
         */
        readonly style: ClientStyle;
        /** The style this client resolves to now: frozen, with the layer each value came from in `origin`. */
        readonly computedStyle: ComputedStyle;
        /** What of the client's style is not drawn and why, and the effects the compositor skipped. */
        readonly styleWarnings: readonly string[];
        /**
         * The room the window's frame takes on each side of its content, in pixels: 0 on every side for a window with no
         * frame or a fullscreen one. `bounds` is the content's rectangle; the frame a layout places is the content with
         * this much added on each side (see docs/js-window-frame.md).
         */
        readonly frameInset: FrameInset;
        /**
         * Whether the window floats on a workspace whose layout tiles: a floating client is not given to the layout's
         * `compute`, so it keeps the place it has. Only the script knows this; the compositor does not.
         */
        floating: boolean;
        /**
         * The rectangle the window floats at: where the user last put it, which a layout that tiles does not take from it. Null
         * while none is kept. The library keeps it, so a window's refit to another output moves it too.
         */
        floatingBounds: Rect | null;
        /** The workspace the client is on, or null. */
        readonly workspace: Workspace | null;
        /** The output that shows the client's workspace, or null while no output shows it (or the output is gone). */
        readonly output: Output | null;
        /**
         * Moves the window to `workspace` and fits it to that workspace's work area: a maximised window fills it, a
         * snapped one re-snaps, a floating one is shrunk to fit and moved inside it. A window that leaves the shown
         * workspace hands its focus to a window that stays. Its dialogs go with it. Refused (false, and nothing moves)
         * for a fullscreen window, a dialog (it moves only with its parent), a dead window, a workspace that is not in
         * `gnoblin.workspaces`, or the workspace it is on already.
         */
        moveToWorkspace(workspace: Workspace): boolean;
        /** `moveToWorkspace` to the workspace `output` shows. False when the output is gone or shows none. */
        moveToOutput(output: Output): boolean;
        /**
         * Starts moving the window with the arrow keys, 16 pixels a press, inside the work area: Return keeps the place,
         * Escape puts the window back. Refused (false) for a window that is fullscreen, maximised, tiled or not showing;
         * a snapped window is unsnapped first. See `gnoblin-std/reposition`.
         */
        moveWithKeys(): boolean;
        /** Starts resizing the window with the arrow keys: Right and Down grow it, Left and Up shrink it. */
        resizeWithKeys(): boolean;
        /**
         * Assigns `props` as one `requestTransition`: the same as
         * `gnoblin.requestTransition(() => { client.opacity = ... })` for this client. Without `options` the change is
         * atomic and immediate. `options.duration` is in milliseconds.
         */
        set(props: Record<string, unknown>, options?: TransitionOptions): Transition;
    }
}
declare module "gnoblin-core/host/output" {
    interface Output {
        /**
         * What a layout tiles in on this output: the output less the bars on it. Null for a gone output or one with no
         * size.
         */
        readonly area: Rect | null;
        /** The workspace the output shows, or null when it shows none or is gone. */
        readonly workspace: Workspace | null;
        /**
         * Shows `workspace` (itself or its id) on this output. A workspace another output shows moves here and that
         * output takes the workspace this one showed, so no workspace is on two outputs. `options.duration` slides the
         * windows.
         */
        show(workspace: Workspace | string, options?: ShowOptions): void;
    }
}
declare module "gnoblin-core/host/collections" {
    interface Collection<T extends HostMember> {
        /**
         * The default styles applied to clients in this collection: `gnoblin.clients.style`, a workspace's `clients`, or
         * a `where()` selection. Only set properties override inherited values; see `Client.style`. Assigning replaces
         * the properties set by this scope.
         */
        get style(): ClientStyle;
        set style(value: ClientStyle | ClientStyleInit);
    }
}
declare module "gnoblin-core/host/roots" {
    interface OutputCollection {
        /**
         * The persistent hardware settings: an `OutputProfile` applies when its monitor is connected, also after a
         * hotplug.
         */
        readonly profiles: MutableCollection<OutputProfile>;
    }
}
declare module "gnoblin-core/host/listeners" {
    interface ClientListener {
        /**
         * The client asked for a change of its window (`request.kind`): maximize, fullscreen, minimize, activate, move
         * and so on. Call `request.handle()` to answer it yourself; the first listener to do so wins and the built-in
         * answer (`windowState`) does not run. The listeners of the client itself are asked before those of the
         * collections.
         */
        onRequest?(client: Client, request: ClientRequest<Client>): void;
    }
}
declare module "gnoblin-core/host/gnoblin" {
    interface Gnoblin {
        /** The optional extensions: `gnoblin.extensions.xr` is null until the XR program has a session (docs/xr.md). */
        readonly extensions: Extensions;
        /** Workspaces the script has attached, in their display order. */
        readonly workspaces: WorkspaceCollection;
        /** The logical displays: what each monitor shows, and through which camera. See `Display`. */
        readonly displays: DisplayCollection;
        /** The scene: its `root` is where a script mounts the nodes it makes (`Node`, `View`, `Camera3D`). */
        readonly scene: Scene;
        /** The touchpad gesture actions (`workspace.next` and four more) and `onSwipe`, `onPinch` and `direction`. */
        readonly gestures: GestureTools;
        /**
         * Runs `callback` once, now, with every property write staged, and draws the change as one animation: each
         * written property moves from where it is drawn to the staged value over `options.duration` milliseconds. The
         * callback also holds layout changes (`workspace.layout = ...`), which move every window from its place. A
         * failure in the callback leaves the previous state. Nothing animates under reduced motion.
         */
        requestTransition(callback: () => void, options?: TransitionOptions): Transition;
        /**
         * Shows a workspace, given as itself or by id, on the output under the pointer. See `WorkspaceCollection.show`.
         */
        show(workspace: Workspace | string, options?: ShowOptions): void;
        find(kind: "Workspace", filter?: FindFilter<Workspace>): Workspace[];
    }
}
declare module "gnoblin-core/input/context" {
    interface InputContext {
        /**
         * The display under the pointer, or the primary display when the pointer is outside every output. Returns null
         * when no display has an output. Assigning `viewport.content` switches that display's workspace.
         */
        readonly activeDisplay: Display | null;
    }
}
/** Names the fields `where` selects on for the kinds of member std adds. Before the roots, which copy them. */
export declare function registerQueryKeys(): void;
/** Adds the members of this package to the core classes and to the `gnoblin` object. */
export declare function extendObjectModel(): void;
