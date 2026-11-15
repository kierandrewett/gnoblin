// The shared state of the object model: one object that the host modules read and write, in place of the
// module-level maps, queues and flags that used to tie them into a single file.

import type { AuthenticationManager } from "gnoblin/authentication";
import type { DialogManager } from "gnoblin/dialogs";
import type { DisplayManager } from "gnoblin/display_manager";
import type { FocusPolicy } from "gnoblin/focus";
import type { GestureTools } from "gnoblin/gesture_actions";
import type { Client } from "gnoblin/host/client";
import type { CollectionState } from "gnoblin/host/collection_state";
import type { ErrorReport, Verdict } from "gnoblin/host/contract";
import type { Desk } from "gnoblin/host/desks";
import type { Invocation, Task } from "gnoblin/host/dispatch";
import type { NodeState } from "gnoblin/host/node_state";
import type { Output, OutputSeen } from "gnoblin/host/output";
import type { PendingApplication } from "gnoblin/host/output_apply";
import type { Slide } from "gnoblin/host/show";
import type { ClientStyleState, ScopeStyle } from "gnoblin/host/style_scopes";
import type { Workspace } from "gnoblin/host/workspace";
import type { WorkspaceState } from "gnoblin/host/workspace_state";
import type { IdleMonitor } from "gnoblin/idle";
import type { InputManager } from "gnoblin/input/manager";
import type { LaunchFeedback } from "gnoblin/launches";
import type { HostApi } from "gnoblin/session/host_api";
import type { Decorations } from "gnoblin/titlebar";
import type { TransitionEngine } from "gnoblin/transition/engine";
import type { Staging } from "gnoblin/transition/staging";
import type { NodeId } from "gnoblin/types";
import type { XRRuntime } from "gnoblin/xr/runtime";

/**
 * Everything the object model keeps between calls. The modules of `gnoblin/host/*` read and write this one object and
 * keep no state of their own: an ES module cannot hand a reassignable `let` to another, so state that several modules
 * change has to live in something they share, and what a generation owns is listed in one place.
 *
 * A generation is a fresh runtime that evaluates this module once, so there is one instance, `ctx`, exported below. It
 * is not passed as a parameter because a script makes `new Workspace(...)` itself, and has nothing to pass.
 */
export class HostContext {
    /** Listener calls and internal tasks waiting for the shell pump. */
    readonly queue: (Invocation | Task)[] = [];

    /** Failures collected for the shell to take after processing callbacks. */
    reports: ErrorReport[] = [];

    /** The texts `takeNotices` has not handed out yet, and every text already said this generation. */
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

    /** The workspace each client is on. A workspace is the script's, so the compositor does not know. */
    readonly membership = new WeakMap<Client, Workspace>();

    /** The clients that were there when the shell booted: a layout does not place them again. */
    readonly preexisting = new WeakSet<Client>();

    /** The clients a layout was offered as arrivals: a dialog never is, so a layout is never told one left. */
    readonly offered = new WeakSet<Client>();

    /** The dialogs already centred in their parent. */
    readonly centred = new WeakSet<Client>();

    /** The views with listeners: the only ones whose membership the plan keeps up to date. */
    readonly tracked: CollectionState[] = [];

    /** All live desks, with reverse lookup by output for active workspaces. */
    readonly desks: Desk[] = [];

    /** Reverse index from an output to the desk it displays. */
    readonly deskByOutput = new WeakMap<Output, Desk>();

    /** Slides waiting to start after the next successful layout pass. */
    readonly slides: Slide[] = [];

    /** The workspaces that are not shown but keep their windows drawn until a slide away from them ends. */
    readonly holding = new Map<Workspace, number>();

    /** Whether each client's layout wants it shown, from the last placement it had. Default shown. */
    readonly shownByLayout = new WeakMap<Client, boolean>();

    /**
     * The `visible` the layout driver last decided for each client. A client it never hid is shown. It is what it
     * compares with, so a window the compositor hid itself (a minimised one) is not shown by a layout that has not
     * changed its mind.
     */
    readonly writtenVisible = new WeakMap<Client, boolean>();

    /** What `barHeight` was when a window was last fitted, so the next pass knows a bar came or went. */
    readonly fittedBar = new WeakMap<Client, number>();

    /** Styles attached to scopes, keyed by the collection state they affect. */
    readonly scopeStyles = new WeakMap<CollectionState, ScopeStyle>();

    /** The `where()` selections that have a style, in the order they got it. */
    readonly styledSelections: CollectionState[] = [];

    /** Per-client style resolution and last-applied host values. */
    readonly clientStyles = new WeakMap<Client, ClientStyleState>();

    /** Pending applications indexed by the compositor's request token. */
    readonly applications = new Map<number, PendingApplication>();

    /** The stock focus policy, attached for every config: a plain `init.ts` has click focus and the built-in snapping. */
    focusPolicy: FocusPolicy<Client> | null = null;

    /** Optional decoration manager, created by `startDecorations`. */
    decorations: Decorations | null = null;

    /** The few values that are reassigned, and so cannot be module-level `let`s. */
    readonly flags = {
        /**
         * False until the first frame: the entry module is still adding workspaces, and the windows that were
         * there at boot join one only when it is done, or a reload could not put them back where they were.
         */
        opened: false,
        /** Whether the layout pass is running, which a transition cannot start inside: its writes are the pass's own. */
        laying: false,
        /** The window that had the first seat's keyboard focus when the last frame ended: what a closing window is checked against. */
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
