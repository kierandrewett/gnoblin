// What `gnoblin-std` keeps for one runtime generation: the workspaces and the desks that show them, the styles, the
// transition engine, the frame decorations and the rest of the desktop policy. `gnoblin-core` keeps the object model in
// its own context (`gnoblin-core/host/context`) and knows nothing of this one; `gnoblin-std/host/install` fills it.
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
import type { Desk } from "gnoblin-std/host/desks";
import type { Slide } from "gnoblin-std/host/show";
import type { ClientStyleState, ScopeStyle } from "gnoblin-std/host/style_scopes";
import type { Workspace } from "gnoblin-std/host/workspace";
import type { WorkspaceState } from "gnoblin-std/host/workspace_state";
import type { TransitionEngine } from "gnoblin-std/transition/engine";
import type { Staging } from "gnoblin-std/transition/staging";

/** The state of the desktop policy for one generation. */
export class StdContext {
    /** Per-workspace state; a WeakMap avoids retaining removed workspaces. */
    readonly wsStates = new WeakMap<object, WorkspaceState>();

    /** The workspace each client belongs to; this is managed by the script, not the compositor. */
    readonly membership = new WeakMap<Client, Workspace>();

    /** Clients offered to a layout as arrivals; dialogs are excluded and do not later leave it. */
    readonly offered = new WeakSet<Client>();

    /** The dialogs already centred in their parent. */
    readonly centred = new WeakSet<Client>();

    /** The clients the script marked floating: a layout leaves them where they are. */
    readonly floating = new WeakSet<Client>();

    /** All live desks, with a reverse lookup from each output. */
    readonly desks: Desk[] = [];

    /** Reverse index from an output to the desk it displays. */
    readonly deskByOutput = new WeakMap<Output, Desk>();

    /** Slides waiting to start after the next successful layout pass. */
    readonly slides: Slide[] = [];

    /** Unshown workspaces whose windows stay drawn until a slide away from them ends. */
    readonly holding = new Map<Workspace, number>();

    /** Whether the last layout placement wants each client shown. Defaults to shown. */
    readonly shownByLayout = new WeakMap<Client, boolean>();

    /**
     * Visibility last set by the layout driver. A client it has not hidden is shown. Comparing against this value
     * prevents a client hidden by the compositor (for example, when minimised) from being shown again until the layout
     * changes its decision.
     */
    readonly writtenVisible = new WeakMap<Client, boolean>();

    /** The frame inset last used to fit a window, so later passes can detect changes. */
    readonly fittedInset = new WeakMap<Client, FrameInset>();

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
        /**
         * Whether the layout pass is running, which a transition cannot start inside: its writes are the pass's own.
         */
        laying: false,
        /** The first-seat focused window at the end of the last frame, used when checking a closing window. */
        lastFocus: null as Client | null,
        /** Stable tie-break order for style selections created at equal priority. */
        styleOrder: 0,
    };

    // What the install makes once, when the package is imported: the layer in front of the host's functions, the root
    // collections and the services. A module reads them as it reads any other field.

    /** The transition layer that stands in front of the host's reads and writes (`installStaging`). */
    staging!: Staging;
    /** The state of `gnoblin.outputs.profiles`. */
    profileRoot!: CollectionState;
    /** The state of `gnoblin.workspaces`. */
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
