// Shared state used by the host modules for one runtime generation.

import type { AuthenticationManager } from "gnoblin-core/authentication";
import type { DialogManager } from "gnoblin-core/dialogs";
import type { DisplayManager } from "gnoblin-std/display_manager/manager";
import type { FocusPolicy } from "gnoblin-std/focus";
import type { Decorations } from "gnoblin-std/frame/decorations";
import type { FrameInset } from "gnoblin-core/inset";
import type { GestureTools } from "gnoblin-std/gesture_actions";
import type { Client } from "gnoblin-core/host/client";
import type { CollectionState } from "gnoblin-core/host/collection_state";
import type { ErrorReport, Verdict } from "gnoblin-core/host/contract";
import type { Desk } from "gnoblin-std/host/desks";
import type { Invocation, Task } from "gnoblin-core/host/dispatch";
import type { NodeState } from "gnoblin-core/host/node_state";
import type { Output, OutputSeen } from "gnoblin-core/host/output";
import type { PendingApplication } from "gnoblin-core/host/output_apply";
import type { Slide } from "gnoblin-std/host/show";
import type { ClientStyleState, ScopeStyle } from "gnoblin-std/host/style_scopes";
import type { Workspace } from "gnoblin-std/host/workspace";
import type { WorkspaceState } from "gnoblin-std/host/workspace_state";
import type { IdleMonitor } from "gnoblin-core/idle";
import type { InputManager } from "gnoblin-core/input/manager";
import type { LaunchFeedback } from "gnoblin-core/launches";
import type { HostApi } from "gnoblin-core/session/host_api";
import type { TransitionEngine } from "gnoblin-std/transition/engine";
import type { Staging } from "gnoblin-std/transition/staging";
import type { NodeId } from "gnoblin-core/types";
import type { XRRuntime } from "gnoblin-std/extensions/xr/runtime";

/**
 * Shared state for one generation of the host object model. Host modules read and write this instance rather than
 * keeping module-local state that other modules cannot update.
 *
 * A generation is a fresh runtime that evaluates this module once, creating the single `ctx` instance exported below.
 * It is not passed as a parameter because scripts construct objects such as `new Workspace(...)` themselves.
 */
export class HostContext {
    /** Listener calls and internal tasks waiting for the shell pump. */
    readonly queue: (Invocation | Task)[] = [];

    /** Failures collected for the shell to take after processing callbacks. */
    reports: ErrorReport[] = [];

    /** Notices waiting to be delivered, plus every notice already emitted this generation. */
    readonly notices: string[] = [];

    /** Notice texts already emitted, to avoid repeating the same style warning. */
    readonly noticed = new Set<string>();

    /** Dialog decisions accumulated for the shell to consume. */
    verdicts: Verdict[] = [];

    /** Per-object state store; a WeakMap preserves object lifetime without retaining abandoned wrappers. */
    readonly states = new WeakMap<object, NodeState>();

    /** Per-workspace state; a WeakMap avoids retaining removed workspaces. */
    readonly wsStates = new WeakMap<object, WorkspaceState>();

    /** Per-output event baseline, retained while the wrapper exists. */
    readonly outputSeen = new WeakMap<object, OutputSeen>();

    /** Collection backing state; the weak keys follow the lifetime of collection objects. */
    readonly collections = new WeakMap<object, CollectionState>();

    /** The stable wrapper for each live host window node. */
    readonly clientsByNode = new Map<NodeId, Client>();

    /** Maps compositor screen-node handles to their stable output wrappers. */
    readonly outputsByNode = new Map<NodeId, Output>();

    /** The workspace each client belongs to; this is managed by the script, not the compositor. */
    readonly membership = new WeakMap<Client, Workspace>();

    /** The clients that were there when the shell booted: a layout does not place them again. */
    readonly preexisting = new WeakSet<Client>();

    /** Clients offered to a layout as arrivals; dialogs are excluded and do not later leave it. */
    readonly offered = new WeakSet<Client>();

    /** The dialogs already centred in their parent. */
    readonly centred = new WeakSet<Client>();

    /** Views with listeners, whose membership is kept current. */
    readonly tracked: CollectionState[] = [];

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

    /** Pending applications indexed by the compositor's request token. */
    readonly applications = new Map<number, PendingApplication>();

    /** The stock focus policy, attached to each config with click focus and built-in snapping. */
    focusPolicy: FocusPolicy<Client> | null = null;

    /** Optional decoration manager, created by `startDecorations`. */
    decorations: Decorations | null = null;

    /** Shared values that need reassignment and cannot be module-level `let` bindings. */
    readonly flags = {
        /**
         * False until the first frame: the entry module is still adding workspaces. Existing windows join a workspace
         * only afterwards, so reloads can restore their previous placement.
         */
        opened: false,
        /** Whether the layout pass is running, which a transition cannot start inside: its writes are the pass's
         * own.
         */
        laying: false,
        /** The first-seat focused window at the end of the last frame, used when checking a closing window. */
        lastFocus: null as Client | null,
        /** Stable tie-break order for style selections created at equal priority. */
        styleOrder: 0,
        /** Monotonically increasing ids for operations exposed through the session API. */
        operationIds: 0,
    };

    // What the facade installs once, before the shell runs anything: the layer in front of the host's functions, the
    // root collections and the services. A module reads them as it reads any other field.

    /** The transition layer that stands in front of the host's reads and writes (`installStaging`). */
    staging!: Staging;
    /** The state of `gnoblin.clients` (`installRoots`). */
    clientRoot!: CollectionState;
    /** The state of `gnoblin.outputs.profiles`. */
    profileRoot!: CollectionState;
    /** The state of `gnoblin.outputs`. */
    outputRoot!: CollectionState;
    /** The state of `gnoblin.workspaces`. */
    workspaceRoot!: CollectionState;
    /** The state of `gnoblin.displays`. */
    displayRoot!: CollectionState;
    /** Reconciles the script's profiles and displays with the connected outputs (`installDisplays`). */
    displayManager!: DisplayManager;
    /** The seats of the compositor as input contexts (`installInput`). */
    input!: InputManager;
    /** `gnoblin.gestures`: the stock actions and the helpers that bind them (`installPolicy`). */
    gestureTools!: GestureTools;
    /** Policy over the compositor's own dialogs (`installServices`). */
    dialogs!: DialogManager;
    /** Idle watches and holds (`installServices`). */
    idleMonitor!: IdleMonitor;
    /** The session's polkit authentications (`installServices`). */
    authentication!: AuthenticationManager;
    /** Launch feedback: the busy state between asking for an application and its window (`installServices`). */
    launches!: LaunchFeedback;
    /** The runtime of the optional XR extension (`installScene`). */
    xrRuntime!: XRRuntime;
    /** The session API: processes, timers, the bus (`installHostApi`). */
    hostApi!: HostApi;
    /** The engine behind `requestTransition` (`installTransitions`). */
    transitions!: TransitionEngine;
}

/** The state of this generation of the object model. */
export const ctx = new HostContext();
