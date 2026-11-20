import type { AuthenticationManager } from "gnoblin-core/authentication";
import type { DialogManager } from "gnoblin-core/dialogs";
import type { Client } from "gnoblin-core/host/client";
import type { CollectionState } from "gnoblin-core/host/collection_state";
import type { ErrorReport, Verdict } from "gnoblin-core/host/contract";
import type { Invocation, Task } from "gnoblin-core/host/dispatch";
import type { NodeState } from "gnoblin-core/host/node_state";
import type { Output, OutputSeen } from "gnoblin-core/host/output";
import type { PendingApplication } from "gnoblin-core/host/output_apply";
import type { IdleMonitor } from "gnoblin-core/idle";
import type { InputManager } from "gnoblin-core/input/manager";
import type { LaunchFeedback } from "gnoblin-core/launches";
import type { HostApi } from "gnoblin-core/session/host_api";
/**
 * Shared state for one generation of the object model `gnoblin-core` builds: clients, outputs, collections and the
 * services. Host modules read and write this instance rather than keeping module-local state that other modules cannot
 * update. A package built on core keeps its own state in its own context (`gnoblin-std/host/state`), not here.
 *
 * A generation is a fresh runtime that evaluates this module once, creating the single `ctx` instance exported below.
 * It is not passed as a parameter because scripts construct their own objects, such as a `Workspace`, themselves.
 */
export declare class HostContext {
    /** Listener calls and internal tasks waiting for the shell pump. */
    readonly queue: (Invocation | Task)[];
    /** Failures collected for the shell to take after processing callbacks. */
    reports: ErrorReport[];
    /** Notices waiting to be delivered, plus every notice already emitted this generation. */
    readonly notices: string[];
    /** Notice texts already emitted, to avoid repeating the same style warning. */
    readonly noticed: Set<string>;
    /** Dialog decisions accumulated for the shell to consume. */
    verdicts: Verdict[];
    /** Per-object state store; a WeakMap preserves object lifetime without retaining abandoned wrappers. */
    readonly states: WeakMap<object, NodeState>;
    /** Per-output event baseline, retained while the wrapper exists. */
    readonly outputSeen: WeakMap<object, OutputSeen>;
    /** Collection backing state; the weak keys follow the lifetime of collection objects. */
    readonly collections: WeakMap<object, CollectionState>;
    /** The stable wrapper for each live host window node. */
    readonly clientsByNode: Map<number, Client>;
    /** Maps compositor screen-node handles to their stable output wrappers. */
    readonly outputsByNode: Map<number, Output>;
    /** The clients that were there when the shell booted: a layout does not place them again. */
    readonly preexisting: WeakSet<Client>;
    /** Views with listeners, whose membership is kept current. */
    readonly tracked: CollectionState[];
    /** Pending applications indexed by the compositor's request token. */
    readonly applications: Map<number, PendingApplication>;
    /** Shared values that need reassignment and cannot be module-level `let` bindings. */
    readonly flags: {
        /**
         * False until the first frame: the entry module is still running. Windows that were there at boot are given
         * to a package that places them only afterwards, so reloads can restore their previous placement.
         */
        opened: boolean;
        /** Monotonically increasing ids for operations exposed through the session API. */
        operationIds: number;
    };
    /** The state of `gnoblin.clients` (`installRoots`). */
    clientRoot: CollectionState;
    /** The state of `gnoblin.outputs`. */
    outputRoot: CollectionState;
    /** The seats of the compositor as input contexts (`installInput`). */
    input: InputManager;
    /** Policy over the compositor's own dialogs (`installServices`). */
    dialogs: DialogManager;
    /** Idle watches and holds (`installServices`). */
    idleMonitor: IdleMonitor;
    /** The session's polkit authentications (`installServices`). */
    authentication: AuthenticationManager;
    /** Launch feedback: the busy state between asking for an application and its window (`installServices`). */
    launches: LaunchFeedback;
    /** The session API: processes, timers, the bus (`installHostApi`). */
    hostApi: HostApi;
}
/** The state of this generation of the object model. */
export declare const ctx: HostContext;
