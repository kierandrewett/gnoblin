import type { OutputConfig } from "gnoblin-core/output_config";
import type { OutputListener } from "gnoblin-core/host/listeners";
import type { Operation } from "gnoblin-core/operation";
import type { Subscriptions } from "gnoblin-core/subscriptions";
import { NativeObject } from "gnoblin-core/native_object";
import type { NodeId, Rect } from "gnoblin-core/types";
/** The native output properties retained after an output disconnects. */
export declare const OUTPUT_KEYS: string[];
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
export declare function publishedOutput(node: NodeId): OutputSeen;
/**
 * A connected output, the compositor's `screen` node: a physical or virtual display with its place in the layout. It
 * shows one workspace at a time (`workspace`, `show`), and the work area the layouts tile in is its rectangle less the
 * bars on it. It is made and destroyed by the compositor as displays come and go; a destroyed one keeps its last values
 * and reads `connected === false`.
 */
export declare class Output extends NativeObject {
    constructor(token: symbol, node: NodeId);
    /** The compositor's label for this output, or `?` when it has none. */
    get id(): string;
    /** Whether the output is still connected. The same as `connected`. */
    get alive(): boolean;
    /** Whether the display is still connected: false once the compositor destroyed the output. */
    get connected(): boolean;
    /** The name the compositor gives the output, which a workspace set's entry and `gnoblin.outputs.where` use. */
    get name(): string;
    /** The monitor's manufacturer, as it reports it. */
    get make(): string;
    /** The monitor's model name, as it reports it. */
    get model(): string;
    /** The connector the display is plugged into (`DP-1`): what an `OutputProfile` matches and `mirrorOf` names. */
    get connector(): string;
    /** The monitor's EDID identity (`MAN:product:serial`), empty for a display that reports none. */
    get identity(): string;
    /** The output this one shows an exact copy of, or null when it shows its own picture. */
    get mirrorOf(): Output | null;
    /** The mode being driven, `WxH@Hz`. */
    get mode(): string;
    /** The factor the output composes at: 2 is a HiDPI panel. */
    get scale(): number;
    /**
     * The output is composed at this multiple of its mode and shown at the mode. 1 is none.
     */
    get supersample(): number;
    /** The size the output is composed at, `mode * supersample`, in pixels: the size of its screenshots. */
    get virtualSize(): {
        readonly width: number;
        readonly height: number;
    };
    /** The scale clients are told: `scale * supersample`. */
    get effectiveScale(): number;
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
    };
    /** The output's rotation and flip, in the compositor's names: `normal`, `90`, `180`, `270` or a `flipped` form. */
    get transform(): string;
    /** Whether this is the primary output, the one the stock shell's bars and `gnoblin.workspaces.area` are on. */
    get primary(): boolean;
    /** Whether the output is switched on. */
    get enabled(): boolean;
    /** The output's left edge in the compositor's coordinates, in logical pixels. */
    get x(): number;
    /** The output's top edge in the compositor's coordinates, in logical pixels. */
    get y(): number;
    /** The output's width in logical pixels. */
    get width(): number;
    /** The output's height in logical pixels. */
    get height(): number;
    /** The output's top-left corner as `{ x, y }`, in logical pixels. */
    get position(): {
        x: number;
        y: number;
    };
    /**
     * Configures the display through the compositor's transactional output engine and keeps the result as this
     * monitor's profile, so the same monitor on the same port gets it back after an unplug. A key left out keeps its
     * value. The whole arrangement is checked before hardware is touched; if the backend then refuses a mode, the
     * earlier changes are put back and the last good arrangement stays. A bad value throws here, naming the key.
     *
     * The compositor answers a frame later, and the returned operation settles with it: it succeeds with this output,
     * or fails with the compositor's own words as `error.message` (they name the key, as `position: HDMI-1 and DP-1
     * would overlap`) and the code `denied`, `unsupported` (a `transform` it cannot draw) or `not_found` (the output
     * is gone). An `OutputListener` hears the same answer in `onApplied`.
     */
    apply(options: OutputConfig & {
        mirrorOf?: Output | string | null;
    }): Operation<Output>;
    /** Shows an exact copy of `source` (scaled to fit, letterboxed) on this display, or stops mirroring with null. */
    mirror(source: Output | null): Operation<Output>;
    /** The whole output in the compositor's coordinates. */
    get bounds(): Rect;
    /** Attaches a listener to this output. Adding the same listener twice has no effect. */
    addListener(listener: OutputListener): void;
    /** Detaches a listener that `addListener` attached; one that is not attached is ignored. */
    removeListener(listener: OutputListener): void;
    /** The output's label for messages, such as `Output#DP-1`. */
    toString(): string;
}
export interface Output extends Subscriptions<OutputListener> {
}
