// What `gnoblin-std` keeps for one runtime generation: the TypeScript half of the workspaces (the object a script holds, its
// layout and the collections that follow the library's), the styles, the transition engine, the frame decorations and the rest
// of the desktop policy. `gnoblin-core` keeps the object model in its own context (`gnoblin-core/host/context`) and knows
// nothing of this one; `gnoblin-std/host/install` fills it.
//
// The workspaces themselves, the desks that say which output shows which of them, the placement of the windows and the
// policies belong to `libgnoblin-std`: this context holds only the identity of what the library made (a `Workspace` object for
// each workspace handle) and what was last told to it.
//
// Like the core context it is one instance per generation, because a generation is a fresh runtime that evaluates this
// module once.

import type { Client } from "gnoblin-core/host/client";
import type { CollectionState } from "gnoblin-core/host/collection_state";
import type { Output } from "gnoblin-core/host/output";
import type { FrameInset } from "gnoblin-core/inset";
import type { DisplayManager } from "gnoblin-std/display_manager/manager";
import type { XRRuntime } from "gnoblin-std/extensions/xr/runtime";
import type { FocusPolicy } from "gnoblin-std/focus";
import type { Decorations } from "gnoblin-std/frame/decorations";
import type { GestureTools } from "gnoblin-std/gesture_actions";
import type { ClientStyleState, ScopeStyle } from "gnoblin-std/host/style_scopes";
import type { Workspace } from "gnoblin-std/host/workspace";
import type { WorkspaceState } from "gnoblin-std/host/workspace_state";
import type { TransitionEngine } from "gnoblin-std/transition/engine";
import type { Staging } from "gnoblin-std/transition/staging";

/** The state of the desktop policy for one generation. */
export class StdContext {
    /** Per-workspace state; a WeakMap avoids retaining removed workspaces. */
    readonly wsStates = new WeakMap<object, WorkspaceState>();

    /** The `Workspace` object of each workspace the library made, by its handle. */
    readonly byHandle = new Map<number, Workspace>();

    /** The workspaces the library already held when this generation installed (a reload that stays in one core generation): the
     * ones this generation's entry does not make again are removed when it has loaded. */
    readonly inherited = new Set<Workspace>();

    /** The workspace each client was last seen on, as the library's collections said. */
    readonly membership = new WeakMap<Client, Workspace>();

    /** Windows that closed lately, by node, so a layout hears of a window that is already gone from the tree. */
    readonly closed = new Map<number, { client: Client; frame: number }>();

    /** The frame the shell is in, counted from the first one: what `closed` is pruned by. */
    frame = 0;

    /** The room each output's bars last told the library they take, as "top,bottom". */
    readonly insets = new WeakMap<Output, string>();

    /** The frame inset the library last was given for each client. */
    readonly insetSent = new WeakMap<Client, FrameInset>();

    /** Styles attached to scopes, keyed by the collection state they affect. */
    readonly scopeStyles = new WeakMap<CollectionState, ScopeStyle>();

    /** The `where()` selections that have a style, in the order they got it. */
    readonly styledSelections: CollectionState[] = [];

    /** Per-client style resolution and last-applied host values. */
    readonly clientStyles = new WeakMap<Client, ClientStyleState>();

    /** The stock focus policy, attached to each config with click focus and built-in snapping. */
    focusPolicy: FocusPolicy<Client> | null = null;

    /** Optional decoration manager, created by `startDecorations`. */
    decorations: Decorations | null = null;

    /** Shared values that need reassignment and cannot be module-level `let` bindings. */
    readonly flags = {
        /** Stable tie-break order for style selections created at equal priority. */
        styleOrder: 0,
    };

    // What the install makes once, when the package is imported: the layer in front of the host's functions, the root
    // collections and the services. A module reads them as it reads any other field.

    /** The transition layer that stands in front of the host's reads and writes (`installStaging`). */
    staging!: Staging;
    /** The state of `gnoblin.outputs.profiles`. */
    profileRoot!: CollectionState;
    /** The state of `gnoblin.workspaces`, which follows the library's list of workspaces. */
    workspaceRoot!: CollectionState;
    /** The state of `gnoblin.displays`. */
    displayRoot!: CollectionState;
    /** Reconciles the script's profiles and displays with the connected outputs (`installDisplays`). */
    displayManager!: DisplayManager;
    /** `gnoblin.gestures`: the stock actions and the helpers that bind them (`installPolicy`). */
    gestureTools!: GestureTools;
    /** The runtime of the optional XR extension (`installScene`). */
    xrRuntime!: XRRuntime;
    /** The engine behind `requestTransition` (`installTransitions`). */
    transitions!: TransitionEngine;
}

/** The state of the desktop policy in this generation. */
export const std = new StdContext();
