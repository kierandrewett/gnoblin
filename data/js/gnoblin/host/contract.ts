// The two sides of the bridge, as types: `Host` is what the Rust shell installs as `globalThis.__gnoblinHost`, and
// `Entry` is what this library hands back for the shell to call each frame. Types only, so the shell's contract
// reads in one place and nothing here runs.

import type { DialogEvent } from "gnoblin/dialogs";
import type { ClientPointerEvent } from "gnoblin/host/listeners";
import type { IdleCommand, IdleReading } from "gnoblin/idle";
import type { InputHost } from "gnoblin/input/host";
import type { InputMessage } from "gnoblin/input/messages";
import type { LockBackend } from "gnoblin/lock";
import type { HostValue, NodeId } from "gnoblin/types";
import type { UiBackend } from "gnoblin/ui";

/** A listener or internal task failure retained until the shell takes frame errors. */
export interface ErrorReport {
    site: string;
    error: unknown;
}

/** What the frame's dialog listeners decided about one observed request. */
export interface Verdict {
    request: number;
    deny: boolean;
    reason: string;
}

/** The callbacks and state access the shell uses to drive one generation. */
export interface Entry {
    plan(created: NodeId[], touched: NodeId[], destroyed: NodeId[]): number;
    next(): number;
    pending(): number;
    ready(nodes: NodeId[]): number;
    /** Tells the clients' listeners of the pointer's edges on windows; returns how many calls wait. */
    pointer(events: WindowPointerEvent[]): number;
    /** Hands the frame's claimed chords and gestures to the input matcher; returns how many calls wait. */
    input(messages: InputMessage[]): number;
    /** Cancels what the input matcher has in flight and releases every claim; returns how many calls wait. */
    cancelAll(): number;
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
    /** Runs the stock default (`installDefaults` of `gnoblin/starter`) for a shell that has no user entry to run. */
    loadDefault(): void;
    snapshot(): WorkspaceSnapshot;
    reconcile(previous: WorkspaceSnapshot): void;
    /** The host API's frame: what the host collected since the last one, as JSON; returns how many calls wait. */
    apiFrame(batch: string): number;
    /**
     * When the script next needs a frame although nothing arrives: a reading of the shell's clock in milliseconds,
     * `Infinity` for never, and a reading in the past for now (`gnoblin/wake`).
     */
    wake(): number;
}

/** Native operations and services the Rust shell exposes to this generation. */
export interface Host {
    /** Checks a document patch (JSON) against the document in force: the validator's problems, or undefined. */
    config: { check(patch: string): string | undefined };
    kind(node: NodeId): string | undefined;
    get(node: NodeId, key: string): HostValue | undefined;
    published(node: NodeId, key: string): HostValue | undefined;
    label(node: NodeId): string | undefined;
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
    /** The `tween` nodes a transition's tracks join (`data/js/gnoblin/transition.ts`). */
    tween: {
        create(): NodeId | string;
        destroy(node: NodeId): string | undefined;
    };
    call(node: NodeId, method: string): string | undefined;
    /** Queues the moves that put a window on top of the others. */
    raise(node: NodeId): string | undefined;
    /** Queues the moves that put a window under the others. */
    lower(node: NodeId): string | undefined;
    windows(): NodeId[];
    /** The nodes whose writes the shell gave up on since the last call, each once. */
    lostWrites(): NodeId[];
    screens(): NodeId[];
    /** The screen the main seat's pointer is on, or undefined while the seat names none. */
    pointerScreen(): NodeId | undefined;
    /** The host API (`gnoblin/session`): JSON in, JSON out. */
    api: {
        spawn(argv: string, options: string): string;
        kill(id: number, signal: string): string | undefined;
        begin(verb: string): string;
        status(): string | undefined;
        version(): string;
        signals(): string;
    };
    input: InputHost;
    ui: Omit<UiBackend, "now">;
    /** The scene nodes the XR module makes and finds (`gnoblin/scene`, `gnoblin/xr`). */
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
    lock: LockBackend;
    /** `Output.apply`: the request text goes to the compositor, whose answer comes back under the token. */
    outputs: {
        apply(node: NodeId, text: string): number | undefined;
        takeAnswers(): { token: number; error: string }[];
    };
    entry?: Entry;
}

/** A pointer edge as the host hands it over: the node it is on, `enter`, `motion`, `leave`, `press` or `release`. */
export interface WindowPointerEvent extends ClientPointerEvent {
    node: NodeId;
    kind: string;
}

/** Serializable state needed to reconcile workspaces and retained window geometry after reload. */
export interface WorkspaceSnapshot {
    /** The workspace the primary output showed. */
    active: string | null;
    workspaces: { id: string; clients: number[] }[];
    /** The workspace each output showed, by the output's name. */
    outputs: { output: string; workspace: string }[];
    /** The rectangles windows go back to, which live in the generation and would be lost with it. */
    kept: KeptWindow[];
}

/** What one window keeps beyond where it is, each rectangle as `[x, y, width, height]`. */
export interface KeptWindow {
    node: number;
    /** The rectangle a maximised window had before it was maximised. */
    floating: number[] | null;
    /** The rectangle a fullscreen window had before it was. */
    windowed: number[] | null;
    /** Where the user left a floating window. */
    leftAt: number[] | null;
    /** The zone a snapped window sits in, and the rectangle unsnapping returns it to. */
    zone: string | null;
    snapFloating: number[] | null;
    /**
     * The room the window's frame kept when it was last fitted (top, right, bottom, left), so a reload that changes the
     * frame moves its content.
     */
    inset: number[] | null;
}
