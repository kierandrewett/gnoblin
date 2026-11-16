// `Output`: a connected display, the compositor's `screen` node, with the workspace it shows.

import type { OutputConfig } from "gnoblin-core/output_config";
import { CREATE, host } from "gnoblin-core/host/bridge";
import { ctx } from "gnoblin-core/host/context";
import { effectiveDesk, liveOutputs } from "gnoblin-std/host/desks";
import { assertListener } from "gnoblin-core/host/listeners";
import type { OutputListener } from "gnoblin-core/host/listeners";
import { read, stateOf } from "gnoblin-core/host/node_state";
import { applyOutput } from "gnoblin-core/host/output_apply";
import type { OutputApplication } from "gnoblin-core/host/output_apply";
import { showWorkspace } from "gnoblin-std/host/show";
import type { ShowOptions } from "gnoblin-std/host/show";
import { areaOf } from "gnoblin-std/host/work_area";
import type { Workspace } from "gnoblin-std/host/workspace";
import { NativeObject } from "gnoblin-core/native_object";
import type { NodeId, Rect } from "gnoblin-core/types";

/** The native output properties retained after an output disconnects. */
export const OUTPUT_KEYS = [
    "name",
    "connector",
    "identity",
    "mirror_of",
    "make",
    "model",
    "x",
    "y",
    "width",
    "height",
    "output_scale",
    "supersample",
    "transform",
    "mode",
    "primary",
    "enabled",
];

/** What listeners were last told about an output, to raise an event only for a real change. */
export interface OutputSeen {
    /** Whether the output has no place of its own: it mirrors another or is off. It has no desk then. */
    detached: boolean;
    /** The output's mode as the compositor publishes it (for example `3440x1440@144`). */
    mode: string;
    /** The output's scale factor. */
    scale: number;
    /** The output's left edge, in logical pixels. */
    x: number;
    /** The output's top edge, in logical pixels. */
    y: number;
    /** The output's width, in logical pixels. */
    width: number;
    /** The output's height, in logical pixels. */
    height: number;
}

/** Reads the compositor-published output properties used for connection and change events. */
export function publishedOutput(node: NodeId): OutputSeen {
    return {
        detached: host.published(node, "mirror_of") !== "" || host.published(node, "enabled") === false,
        mode: host.published(node, "mode") as string,
        scale: host.published(node, "output_scale") as number,
        x: host.published(node, "x") as number,
        y: host.published(node, "y") as number,
        width: host.published(node, "width") as number,
        height: host.published(node, "height") as number,
    };
}

/**
 * A connected output, the compositor's `screen` node: a physical or virtual display with its place in the layout. It
 * shows one workspace at a time (`workspace`, `show`), and the work area the layouts tile in is its rectangle less the
 * bars on it. It is made and destroyed by the compositor as displays come and go; a destroyed one keeps its last values
 * and reads `connected === false`.
 */
export class Output extends NativeObject {
    constructor(token: symbol, node: NodeId) {
        super();
        if (token !== CREATE) {
            throw new TypeError("an Output is made by the compositor, not by a script");
        }
        ctx.states.set(this, {
            node,
            kind: "output",
            alive: true,
            frozen: null,
            listeners: [],
            readied: false,
            seen: null,
            floating: false,
        });
        ctx.outputSeen.set(this, publishedOutput(node));
    }

    /** The compositor's label for this output, or `?` when it has none. */
    get id(): string {
        return host.label(stateOf(this).node) ?? "?";
    }

    /** Whether the output is still connected. The same as `connected`. */
    get alive(): boolean {
        return stateOf(this).alive;
    }

    /** Whether the display is still connected: false once the compositor destroyed the output. */
    get connected(): boolean {
        return stateOf(this).alive;
    }

    /** The name the compositor gives the output, which a workspace set's entry and `gnoblin.outputs.where` use. */
    get name(): string {
        return read<string>(this, "name");
    }

    /** The monitor's manufacturer, as it reports it. */
    get make(): string {
        return read<string>(this, "make");
    }

    /** The monitor's model name, as it reports it. */
    get model(): string {
        return read<string>(this, "model");
    }

    /** The connector the display is plugged into (`DP-1`): what an `OutputProfile` matches and `mirrorOf` names. */
    get connector(): string {
        return read<string>(this, "connector");
    }

    /** The monitor's EDID identity (`MAN:product:serial`), empty for a display that reports none. */
    get identity(): string {
        return read<string>(this, "identity");
    }

    /** The output this one shows an exact copy of, or null when it shows its own picture. */
    get mirrorOf(): Output | null {
        const source = read<string>(this, "mirror_of");
        return source === "" ? null : (liveOutputs().find((one) => one.connector === source) ?? null);
    }

    /** The mode being driven, `WxH@Hz`. */
    get mode(): string {
        return read<string>(this, "mode");
    }

    /** The factor the output composes at: 2 is a HiDPI panel. */
    get scale(): number {
        return read<number>(this, "output_scale");
    }

    /**
     * The output is composed at this multiple of its mode and shown at the mode. 1 is none.
     */
    get supersample(): number {
        return read<number>(this, "supersample");
    }

    /** The size the output is composed at, `mode * supersample`, in pixels: the size of its screenshots. */
    get virtualSize(): { readonly width: number; readonly height: number } {
        const [, width, height] = /^(\d+)x(\d+)@/.exec(this.mode) ?? [];
        const factor = this.supersample;
        return { width: Math.round(Number(width) * factor), height: Math.round(Number(height) * factor) };
    }

    /** The scale clients are told: `scale * supersample`. */
    get effectiveScale(): number {
        return this.scale * this.supersample;
    }

    /**
     * The compositor's requested GPU memory, in bytes; client-owned buffers are not counted. `target` is the
     * output-sized target (and the copy a supersampled output averages down into), `scratch` the effect scratch
     * targets, `staging` the host-visible buffers a renderer keeps between frames, `views` the pictures of the Views
     * drawn on the GPU, `capture` the last presented frame the capture protocols copy from, and `total` their sum.
     *
     * The last four are the memory budget's numbers for the whole session, the same on every output: `clients` is
     * the bytes of every client's live buffers, `budget` the budget the compositor holds the clients, the renderers
     * and capture to (zero when it is off), `refused` the requests refused for passing a bound since the session
     * began, and `evicted` the bytes given back from caches to make room for a client's buffer.
     */
    get gpuMemory(): {
        readonly target: number;
        readonly scratch: number;
        readonly staging: number;
        readonly views: number;
        readonly capture: number;
        readonly total: number;
        readonly clients: number;
        readonly budget: number;
        readonly refused: number;
        readonly evicted: number;
    } {
        return {
            target: read<number>(this, "gpu_memory_target_bytes"),
            scratch: read<number>(this, "gpu_memory_scratch_bytes"),
            staging: read<number>(this, "gpu_memory_staging_bytes"),
            views: read<number>(this, "gpu_memory_views_bytes"),
            capture: read<number>(this, "gpu_memory_capture_bytes"),
            total: read<number>(this, "gpu_memory_bytes"),
            clients: read<number>(this, "gpu_memory_clients_bytes"),
            budget: read<number>(this, "gpu_memory_budget_bytes"),
            refused: read<number>(this, "gpu_memory_refused_total"),
            evicted: read<number>(this, "gpu_memory_evicted_total"),
        };
    }

    /** The output's rotation and flip, in the compositor's names: `normal`, `90`, `180`, `270` or a `flipped` form. */
    get transform(): string {
        return read<string>(this, "transform");
    }

    /** Whether this is the primary output, the one the stock shell's bars and `gnoblin.workspaces.area` are on. */
    get primary(): boolean {
        return read<boolean>(this, "primary");
    }

    /** Whether the output is switched on. */
    get enabled(): boolean {
        return read<boolean>(this, "enabled");
    }

    /** The output's left edge in the compositor's coordinates, in logical pixels. */
    get x(): number {
        return read<number>(this, "x");
    }

    /** The output's top edge in the compositor's coordinates, in logical pixels. */
    get y(): number {
        return read<number>(this, "y");
    }

    /** The output's width in logical pixels. */
    get width(): number {
        return read<number>(this, "width");
    }

    /** The output's height in logical pixels. */
    get height(): number {
        return read<number>(this, "height");
    }

    /** The output's top-left corner as `{ x, y }`, in logical pixels. */
    get position(): { x: number; y: number } {
        return { x: this.x, y: this.y };
    }

    /**
     * Configures the display through the compositor's transactional output engine and keeps the result as this
     * monitor's profile, so the same monitor on the same port gets it back after an unplug. A key left out keeps its
     * value. The whole arrangement is checked before hardware is touched; if the backend then refuses a mode, the
     * earlier changes are put back and the last good arrangement stays. A bad value throws here, naming the key.
     * The compositor's answer arrives a frame later on the returned `OutputApplication` and `onApplied`.
     */
    apply(options: OutputConfig & { mirrorOf?: Output | string | null }): OutputApplication {
        return applyOutput(this, options, `${this}.apply`);
    }

    /** Shows an exact copy of `source` (scaled to fit, letterboxed) on this display, or stops mirroring with null. */
    mirror(source: Output | null): OutputApplication {
        return applyOutput(this, { mirrorOf: source }, `${this}.mirror`);
    }

    /** The whole output in the compositor's coordinates. */
    get bounds(): Rect {
        return { x: this.x, y: this.y, width: this.width, height: this.height };
    }

    /**
     * What a layout tiles in on this output: the output less the bars on it. Null for a gone output or one with no
     * size.
     */
    get area(): Rect | null {
        return stateOf(this).alive ? areaOf(this) : null;
    }

    /** The workspace the output shows, or null when it shows none or is gone. */
    get workspace(): Workspace | null {
        const desk = ctx.deskByOutput.get(this);
        return desk !== undefined && stateOf(this).alive ? effectiveDesk(desk).shown : null;
    }

    /**
     * Shows `workspace` (itself or its id) on this output. A workspace another output shows moves here and that output
     * takes the workspace this one showed, so no workspace is on two outputs. `options.duration` slides the windows.
     */
    show(workspace: Workspace | string, options: ShowOptions = {}): void {
        showWorkspace(workspace, { ...options, output: this });
    }

    /** Attaches a listener to this output. Adding the same listener twice has no effect. */
    addListener(listener: OutputListener): void {
        assertListener(listener, `${this}.addListener`);
        const { listeners } = stateOf(this);
        if (!listeners.includes(listener)) {
            listeners.push(listener);
        }
    }

    /** Detaches a listener that `addListener` attached; one that is not attached is ignored. */
    removeListener(listener: OutputListener): void {
        const { listeners } = stateOf(this);
        const at = listeners.indexOf(listener);
        if (at >= 0) {
            listeners.splice(at, 1);
        }
    }

    /** The output's label for messages, such as `Output#DP-1`. */
    toString(): string {
        return `Output#${host.label(stateOf(this).node)}`;
    }
}
