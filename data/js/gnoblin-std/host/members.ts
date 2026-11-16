// What `gnoblin-std` adds to the objects of `gnoblin-core`: the members that only make sense with a workspace, a style or
// a transition behind them. `Client`, `Output`, `Collection`, `Gnoblin` and `InputContext` are core classes, so the
// members are put on their prototypes (or, for the data members of `gnoblin`, on the instance) when this package is
// imported, and merged into their types by the declarations below. A shell that imports only `gnoblin-core` has none of
// them, in the types or at run time.

import { NO_INSET } from "gnoblin-core/inset";
import type { FrameInset } from "gnoblin-core/inset";
import { host } from "gnoblin-core/host/bridge";
import { Client } from "gnoblin-core/host/client";
import { collectionState, QUERY_KEYS } from "gnoblin-core/host/collection_state";
import type { Member as HostMember } from "gnoblin-core/host/collection_state";
import { Collection } from "gnoblin-core/host/collections";
import type { MutableCollection } from "gnoblin-core/host/collections";
import { gnoblin } from "gnoblin-core/host/facade";
import { Gnoblin } from "gnoblin-core/host/gnoblin";
import type { FindFilter } from "gnoblin-core/host/gnoblin";
import { isOutput, stateOf } from "gnoblin-core/host/node_state";
import { Output } from "gnoblin-core/host/output";
import { InputContext } from "gnoblin-core/input/context";
import type { Rect } from "gnoblin-core/types";
import { defineSubscriptions, withPrefix } from "gnoblin-core/subscriptions";
import type { ClientListener } from "gnoblin-core/host/listeners";
import type { Display } from "gnoblin-std/display/display";
import type { OutputProfile } from "gnoblin-std/display/output_profile";
import { Extensions } from "gnoblin-std/extensions/xr/availability";
import { deskOfClient, effectiveDesk } from "gnoblin-std/host/desks";
import { moveClientTo } from "gnoblin-std/host/membership";
import type { DisplayCollection, WorkspaceCollection } from "gnoblin-std/host/roots";
import { showWorkspace } from "gnoblin-std/host/show";
import type { ShowOptions } from "gnoblin-std/host/show";
import { std } from "gnoblin-std/host/state";
import { clientStyle, computedStyleOf, scopeStyle, styleWarningsOf } from "gnoblin-std/host/style_scopes";
import { areaOf } from "gnoblin-std/host/work_area";
import type { Workspace } from "gnoblin-std/host/workspace";
import { wsOf } from "gnoblin-std/host/workspace_state";
import { repositioning } from "gnoblin-std/reposition";
import type { RepositionClient } from "gnoblin-std/reposition";
import { sceneOf } from "gnoblin-std/scene/scene";
import type { Scene } from "gnoblin-std/scene/scene";
import type { ComputedStyle } from "gnoblin-std/style/resolve";
import { replaceStyle } from "gnoblin-std/style/style";
import type { ClientStyle, ClientStyleInit } from "gnoblin-std/style/style";
import { setProperties } from "gnoblin-std/transition/engine";
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
    // The declaration merges with `Collection<T>`, whose type parameter it has to repeat.
    // eslint-disable-next-line @typescript-eslint/no-unused-vars
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

/** A member a package adds to a class: a method, or an accessor with its two halves. */
type Extension = ((...args: never[]) => unknown) | { get?(): unknown; set?(value: never): void };

/** Defines `members` on `target` as a class would have: methods and accessors that are not enumerable. */
function extend(target: object, members: Record<string, Extension>): void {
    for (const [name, member] of Object.entries(members)) {
        Object.defineProperty(
            target,
            name,
            typeof member === "function"
                ? { value: member, writable: true, configurable: true }
                : { ...member, configurable: true },
        );
    }
}

/** Puts a data member on an instance the way a class field does: own, enumerable and writable. */
function field(target: object, name: string, value: unknown): void {
    Object.defineProperty(target, name, { value, writable: true, enumerable: true, configurable: true });
}

/** Names the fields `where` selects on for the kinds of member std adds. Before the roots, which copy them. */
export function registerQueryKeys(): void {
    Object.assign(QUERY_KEYS, {
        Workspace: ["id", "name", "index", "active"],
        Display: ["id", "mode"],
        OutputProfile: ["id", "enabled", "scale", "transform"],
    });
}

/** Adds the members of this package to the core classes and to the `gnoblin` object. */
export function extendObjectModel(): void {
    extend(Client.prototype, {
        style: {
            get(this: Client) {
                return clientStyle(this);
            },
        },
        computedStyle: {
            get(this: Client) {
                return computedStyleOf(this);
            },
        },
        styleWarnings: {
            get(this: Client) {
                return styleWarningsOf(this);
            },
        },
        frameInset: {
            get(this: Client): FrameInset {
                return std.decorations?.insetOf(this) ?? NO_INSET;
            },
        },
        floating: {
            get(this: Client): boolean {
                return std.floating.has(this);
            },
            set(this: Client, value: boolean) {
                if (typeof value !== "boolean") {
                    throw new TypeError(`Client#${host.label(stateOf(this).node)}.floating: must be true or false`);
                }
                if (std.floating.has(this) !== value) {
                    if (value) {
                        std.floating.add(this);
                    } else {
                        std.floating.delete(this);
                    }
                    const own = wsOf(this);
                    if (own) {
                        own.dirty = true;
                    }
                }
            },
        },
        workspace: {
            get(this: Client): Workspace | null {
                return std.membership.get(this) ?? null;
            },
        },
        output: {
            get(this: Client): Output | null {
                const desk = deskOfClient(this);
                return desk?.parked === false ? desk.output : null;
            },
        },
        moveToWorkspace(this: Client, workspace: Workspace): boolean {
            return moveClientTo(this, workspace);
        },
        moveToOutput(this: Client, output: Output): boolean {
            if (!isOutput(output)) {
                throw new TypeError(`${this}.moveToOutput: needs an Output`);
            }
            const shown = output.alive ? output.workspace : null;
            return shown !== null && moveClientTo(this, shown);
        },
        moveWithKeys(this: Client): boolean {
            return repositioning.begin(this as unknown as RepositionClient, "move");
        },
        resizeWithKeys(this: Client): boolean {
            return repositioning.begin(this as unknown as RepositionClient, "resize");
        },
        set(this: Client, props: Record<string, unknown>, options?: TransitionOptions): Transition {
            return setProperties(std.transitions, this, `${this}`, props, options);
        },
    });

    extend(Output.prototype, {
        area: {
            get(this: Output): Rect | null {
                return stateOf(this).alive ? areaOf(this) : null;
            },
        },
        workspace: {
            get(this: Output): Workspace | null {
                const desk = std.deskByOutput.get(this);
                return desk !== undefined && stateOf(this).alive ? effectiveDesk(desk).shown : null;
            },
        },
        show(this: Output, workspace: Workspace | string, options: ShowOptions = {}): void {
            showWorkspace(workspace, { ...options, output: this });
        },
    });

    extend(Collection.prototype, {
        style: {
            get(this: Collection<never>): ClientStyle {
                return scopeStyle(collectionState(this)).style;
            },
            set(this: Collection<never>, value: ClientStyle | ClientStyleInit) {
                replaceStyle(scopeStyle(collectionState(this)).style, value);
            },
        },
    });

    // `ClientListener.onRequest` is std's own method, so its named subscriptions are std's too: on a client, on a
    // collection of clients, and on `gnoblin` as `onClientRequest`.
    defineSubscriptions<ClientListener>(Client.prototype, ["onRequest"]);
    defineSubscriptions<ClientListener>(Collection.prototype, ["onRequest"]);
    defineSubscriptions<ClientListener>(Gnoblin.prototype, ["onRequest"], {
        name: withPrefix("Client"),
        target: (self: Gnoblin) => self.clients,
    });

    field(gnoblin.outputs, "profiles", std.profileRoot.self);

    extend(InputContext.prototype, {
        activeDisplay: {
            get(this: InputContext): Display | null {
                return std.displayManager.displayAt(this.pointer);
            },
        },
    });

    extend(Gnoblin.prototype, {
        requestTransition(this: Gnoblin, callback: () => void, options?: TransitionOptions): Transition {
            return std.transitions.request(callback, options);
        },
        show(this: Gnoblin, workspace: Workspace | string, options?: ShowOptions): void {
            showWorkspace(workspace, options);
        },
    });
    field(gnoblin, "extensions", new Extensions(std.xrRuntime));
    field(gnoblin, "workspaces", std.workspaceRoot.self);
    field(gnoblin, "displays", std.displayRoot.self);
    field(gnoblin, "scene", sceneOf());
    field(gnoblin, "gestures", std.gestureTools);
}
