// Types for the bridge between the Rust shell and this library: host functions in, frame callbacks out.

import type { DialogEvent } from "gnoblin/dialogs";
import type { ClientPointerEvent } from "gnoblin/host/listeners";
import type { IdleCommand, IdleReading } from "gnoblin/idle";
import type { InputHost } from "gnoblin/input/host";
import type { InputMessage } from "gnoblin/input/messages";
import type { LockBackend } from "gnoblin/lock";
import type { HostValue, NodeId } from "gnoblin/types";
import type { UiBackend } from "gnoblin/ui/backend";

/** A listener or task failure saved until the shell retrieves frame errors. */
export interface ErrorReport {
    /** Callback name or source that raised the failure. */
    site: string;
    /** Exception thrown by the listener or task. */
    error: unknown;
}

/** The decision made by dialog listeners for an observed request. */
export interface Verdict {
    /** Identifier of the observed request. */
    request: number;
    /** Whether the request was denied. */
    deny: boolean;
    /** Reason supplied with the decision. */
    reason: string;
}

/** Callbacks and state access the shell uses to drive one runtime generation. */
export interface Entry {
    /** Processes created, changed and destroyed host nodes for this frame. */
    plan(created: NodeId[], touched: NodeId[], destroyed: NodeId[]): number;
    /** Runs the next queued callback and returns the queue length. */
    next(): number;
    /** Number of callbacks waiting in the queue. */
    pending(): number;
    /** Marks host nodes whose wrappers are ready and returns pending callback count. */
    ready(nodes: NodeId[]): number;
    /** Tells the clients' listeners of the pointer's edges on windows; returns how many calls wait. */
    pointer(events: WindowPointerEvent[]): number;
    /** Hands the frame's claimed chords and gestures to the input matcher; returns how many calls wait. */
    input(messages: InputMessage[]): number;
    /** Cancels what the input matcher has in flight and releases every claim; returns how many calls wait. */
    cancelAll(): number;
    /** Transfers collected callback failures to the shell. */
    takeErrors(): ErrorReport[];
    /** What the module could not do, said once per generation: a style property no renderer draws. */
    takeNotices(): string[];
    /**
     * The config document patch of the running generation, as JSON text: `{}` when it sets none. It holds the device
     * settings a script wrote (`Mouse.sensitivity` and the like).
     */
    document(): string;
    /** A number that moves when a device setting is written: the shell commits `document()` again when it moved. */
    documentRevision(): number;
    /** What the compositor sent for the dialog observation slot; returns how many calls wait. */
    dialogs(events: DialogEvent[]): number;
    /** The verdicts the frame's listeners gave, each once. */
    takeVerdicts(): Verdict[];
    /** Whether a dialog listener is attached, so the shell holds the observation slot. */
    dialogsWanted(): boolean;
    /** What the script needs of the compositor beyond the tree: bit 1 is idle state. */
    wants(): number;
    /** The compositor's newest idle reading; returns how many calls wait. */
    idleReading(reading: IdleReading): number;
    /** The idle connection ended: every watch is active again; returns how many calls wait. */
    idleLost(): number;
    /** The idle holds the script asked for or gave back, each once. */
    takeIdleCommands(): IdleCommand[];
    /** The names of the idle holds that are alive, for the shell to give back when the generation ends. */
    idleHolders(): string[];
    /** The compositor refused the idle hold `holder`. */
    idleFailed(holder: string): void;
    /** Lays out the workspaces that changed, once each; returns how many it laid out. */
    layout(): number;
    /** Runs the UI toolkit's frame: measurements and pointer events in, every canvas arranged and painted. */
    ui(): void;
    /** Renders a console value for the socket: JSON text of `{ type, value?, text, truncated }`. */
    render(value: unknown, maxBytes?: number): string;
    /** Gives a config that declared no workspace the stock one. */
    finishLoad(): void;
    /** Captures workspaces, output assignments and retained window geometry. */
    snapshot(): WorkspaceSnapshot;
    /** Restores runtime state from a previous generation snapshot. */
    reconcile(previous: WorkspaceSnapshot): void;
    /** The host API's frame: what the host collected since the last one, as JSON; returns how many calls wait. */
    apiFrame(batch: string): number;
    /**
     * When the script next needs a frame although nothing arrives: a reading of the shell's clock in milliseconds,
     * `Infinity` for never, and a reading in the past for now (`gnoblin/wake`).
     */
    wake(): number;
}

/** Native operations and services provided by the Rust shell to this generation. */
export interface Host {
    /** Validates a JSON config patch against the current document; returns a problem or undefined. */
    config: { check(patch: string): string | undefined };
    /** Returns the node kind, or undefined when the node is unknown. */
    kind(node: NodeId): string | undefined;
    /** Reads the current value of a host node property. */
    get(node: NodeId, key: string): HostValue | undefined;
    /** Reads the last value published by the host for a property. */
    published(node: NodeId, key: string): HostValue | undefined;
    /** Returns a node's stable label, or undefined when it has none. */
    label(node: NodeId): string | undefined;
    /** Queues a property write and returns an error code when it is refused. */
    set(node: NodeId, key: string, value: HostValue): string | undefined;
    /** What `set` would answer, without queuing the write; `timed` also refuses a property the tree cannot animate. */
    check(node: NodeId, key: string, value: HostValue, timed: boolean): string | undefined;
    /**
     * The same write, carried out over `duration` milliseconds along `samples` (progress from 0 to 1), after `delay`
     * milliseconds when given, and driven by the `tween` node when one is named (0 is none).
     */
    animate(
        node: NodeId,
        key: string,
        value: HostValue,
        duration: number,
        samples: number[],
        delay?: number,
        tween?: number,
    ): string | undefined;
    /** Tween nodes used by tracks in a transition (`data/js/gnoblin/transition.ts`). */
    tween: {
        create(): NodeId | string;
        destroy(node: NodeId): string | undefined;
    };
    /** Calls a host operation on a node and returns an error code when refused. */
    call(node: NodeId, method: string): string | undefined;
    /** Queues the moves that put a window on top of the others. */
    raise(node: NodeId): string | undefined;
    /** Queues the moves that put a window under the others. */
    lower(node: NodeId): string | undefined;
    /** Returns the current window node handles. */
    windows(): NodeId[];
    /** The nodes whose writes the shell gave up on since the last call, each once. */
    lostWrites(): NodeId[];
    /** Returns the connected screen node handles. */
    screens(): NodeId[];
    /** The screen the main seat's pointer is on, or undefined while the seat names none. */
    pointerScreen(): NodeId | undefined;
    /** The host API (`gnoblin/session`): JSON in, JSON out. */
    api: {
        spawn(argv: string, options: string): string;
        kill(id: number, signal: string): string | undefined;
        pid(id: number): number;
        begin(verb: string): string;
        status(): string | undefined;
        version(): string;
        signals(): string;
    };
    /** Input services provided by the shell. */
    input: InputHost;
    /** UI services provided by the shell, without its clock callback. */
    ui: Omit<UiBackend, "now">;
    /** The scene nodes the XR module makes and finds (`gnoblin/scene`, `gnoblin/extensions/xr`). */
    xr: {
        create(cls: string): NodeId | string;
        insert(node: NodeId, parent: NodeId, index: number): string | undefined;
        destroy(node: NodeId): string | undefined;
        nodes(cls: string): NodeId[];
        children(node: NodeId): NodeId[];
        parent(node: NodeId): NodeId | undefined;
        root(): NodeId;
        declared(): boolean;
    };
    /** Lock-screen services provided by the shell. */
    lock: LockBackend;
    /** `Output.apply` requests and their compositor responses. */
    outputs: {
        apply(node: NodeId, text: string): number | undefined;
        takeAnswers(): { token: number; error: string }[];
    };
    /** Set by the shell to receive callbacks for this generation. */
    entry?: Entry;
}

/** A pointer edge from the host: the node and `enter`, `motion`, `leave`, `press` or `release`. */
export interface WindowPointerEvent extends ClientPointerEvent {
    /** Window node where the pointer event occurred. */
    node: NodeId;
    /** Pointer edge kind, such as `enter`, `motion` or `press`. */
    kind: string;
}

/** Serializable state used to restore workspaces and window geometry after reload. */
export interface WorkspaceSnapshot {
    /** The workspace the primary output showed. */
    active: string | null;
    /** Workspace identifiers and their retained window nodes. */
    workspaces: { id: string; clients: number[] }[];
    /** The workspace each output showed, by the output's name. */
    outputs: { output: string; workspace: string }[];
    /** The rectangles windows go back to, which live in the generation and would be lost with it. */
    kept: KeptWindow[];
}

/** State retained for a window beyond its current placement; rectangles are `[x, y, width, height]`. */
export interface KeptWindow {
    /** Host node handle for the retained window. */
    node: number;
    /** The rectangle a maximised window had before it was maximised. */
    floating: number[] | null;
    /** The rectangle a fullscreen window had before it was. */
    windowed: number[] | null;
    /** Where the user left a floating window. */
    leftAt: number[] | null;
    /** The zone a snapped window sits in, and the rectangle unsnapping returns it to. */
    zone: string | null;
    /** Floating rectangle restored when a snapped window is unsnapped. */
    snapFloating: number[] | null;
    /**
     * The room the window's frame kept when it was last fitted (top, right, bottom, left), so a reload that changes the
     * frame moves its content.
     */
    inset: number[] | null;
}
