import type { ClientListener } from "gnoblin-core/host/listeners";
import type { Subscriptions } from "gnoblin-core/subscriptions";
import { NativeObject } from "gnoblin-core/native_object";
import type { PropertyBinding, PropertyBindings } from "gnoblin-core/properties";
import { Transform3D } from "gnoblin-core/math/transform";
import { Vector3 as Position } from "gnoblin-core/math/vector";
import type { AxisTriple, ScaleInit, TransformInit, Triple, Vec3Like } from "gnoblin-core/math/spatial";
import type { NodeId, Rect } from "gnoblin-core/types";
/** How the compositor answers a window's frame callbacks: see `Client.framePolicy`. */
export type FramePolicy = "auto" | "hold" | "throttle";
/** The host properties frozen when a client leaves, so the object keeps its final readable state. */
export declare const CLIENT_KEYS: string[];
/** A toplevel window the compositor manages. */
export declare class Client extends NativeObject {
    constructor(token: symbol, node: NodeId);
    /** Stable compositor identifier for this window. */
    get id(): string;
    /** Whether the compositor still manages this window. */
    get alive(): boolean;
    /** Application identifier reported for the window. */
    get appId(): string;
    /** Current window title. */
    get title(): string;
    /** Whether this window has keyboard focus. */
    get focused(): boolean;
    /** Whether the window covers its output. Writing asks the compositor to make it fullscreen or restore it. */
    get fullscreen(): boolean;
    /** Requests fullscreen or restores the window. */
    set fullscreen(value: boolean);
    /** Whether the window is kept out of screen captures. */
    get secure(): boolean;
    /** Controls whether the window is excluded from screen captures. */
    set secure(value: boolean);
    /** Whether the client has left a liveness ping unanswered too long. The compositor writes it, and clears it on an
     * answer.
     */
    get unresponsive(): boolean;
    /**
     * Who draws the window's frame, as the compositor negotiated it: `server` when the shell's titlebar does (the
     * client asked for server-side and this shell declared decorations, or the window is an X11 one), `client` when
     * the client draws its own title bar (a GTK, Qt or foot window), `none` for a window nobody frames. The
     * compositor writes it.
     * `frameInset` is taken only for a `server` window.
     */
    get decoration(): string;
    /** What the client asked of `xdg-decoration`: `client`, `server`, or `unset` (it asked nothing, or is an X11
     * window).
     */
    get decorationWanted(): string;
    /**
     * How the compositor answers this window's client when it asks to be told it may draw its next frame: `auto` after
     * every frame the compositor draws that shows the window, `hold` never (the client waits and draws nothing new,
     * which is what a shell wants for a window it does not show), `throttle` at most `frameRate` times a second. A held
     * or throttled window keeps its last picture and is drawn as before. It takes effect only while a shell holds the
     * tree, and the compositor sets every window back to `auto` when the shell lets go of it.
     */
    get framePolicy(): FramePolicy;
    /** Sets how the window's client is answered; any other text than `auto`, `hold` or `throttle` is refused. */
    set framePolicy(value: FramePolicy);
    /**
     * The most frame callbacks a second a `throttle` window's client is answered, in hertz: above 0 and at most 1000.
     * It is read only while `framePolicy` is `throttle`.
     */
    get frameRate(): number;
    /** Sets the rate a `throttle` window's client is answered at, in hertz. */
    set frameRate(value: number);
    /**
     * Whether the compositor tells the shell when the window's pixels reach the screen: while it is true each frame
     * that draws the window writes `presented`. It is false by default, because a window written every frame is a
     * change in every batch.
     */
    get reportPresented(): boolean;
    /** Asks the compositor to report, or stop reporting, when the window's pixels reach the screen. */
    set reportPresented(value: boolean);
    /**
     * When the last frame that showed the window reached the screen, in milliseconds on the compositor's monotonic
     * clock (`CLOCK_MONOTONIC`, which `wp_presentation` names), or 0 until a frame was presented with
     * `reportPresented` true. The compositor writes it.
     */
    get presented(): number;
    /** Horizontal position in logical pixels. */
    get x(): number;
    /** Sets the horizontal position in logical pixels. */
    set x(value: number);
    /** Vertical position in logical pixels. */
    get y(): number;
    /** Sets the vertical position in logical pixels. */
    set y(value: number);
    /** Width in logical pixels. */
    get width(): number;
    /** Sets the width in logical pixels. */
    set width(value: number);
    /** Height in logical pixels. */
    get height(): number;
    /** Sets the height in logical pixels. */
    set height(value: number);
    /** The layout geometry. Assigning writes all four values, or none when one is not a finite number. */
    get bounds(): Rect;
    /** Sets the content rectangle; all four values must be finite numbers. */
    set bounds(rect: Rect);
    /** Whether the window is maximised. Writing asks the compositor to maximise or restore it. */
    get maximised(): boolean;
    /** Requests maximisation or restoration. */
    set maximised(value: boolean);
    /** Whether the window is minimised. Writing asks the compositor to minimise or restore it. */
    get minimised(): boolean;
    /** Requests minimisation or restoration. */
    set minimised(value: boolean);
    /** Whether the window sits in a tiled layout, so its client drops the border and rounding it draws itself. */
    get tiled(): boolean;
    /** Sets whether the client is tiled. */
    set tiled(value: boolean);
    /** Whether the window is drawn and hit. A workspace that is not shown hides its windows. */
    get visible(): boolean;
    /** Sets whether the window is drawn and receives input. */
    set visible(value: boolean);
    /** How opaque the window is drawn, from 0 to 1 (the compositor clamps it). */
    get opacity(): number;
    /** Sets opacity from 0 (transparent) to 1 (opaque); the compositor clamps the value. */
    set opacity(value: number);
    /** The window's place in depth; negative is in front of the camera. */
    get z(): number;
    /** Sets the window's depth; negative values place it in front of the camera. */
    set z(value: number);
    /**
     * Rotation about each axis in radians; the list also answers to `x`, `y` and `z`. Assigning takes a list,
     * `{ x, y, z }` or a quaternion `{ x, y, z, w }`.
     */
    get rotation(): AxisTriple;
    /** Sets rotation about the x, y and z axes, in radians. */
    set rotation(value: Triple | Vec3Like);
    /** Scale along each axis; the list also answers to `x`, `y` and `z`. Assigning takes one number for every axis,
     * a list or `{ x, y, z }`.
     */
    get scale(): AxisTriple;
    /** Sets scale along the x, y and z axes. */
    set scale(value: ScaleInit);
    /** Where the window is: `x`, `y` and its depth `z`. Assigning takes a list or `{ x, y, z }` and writes the
     * three.
     */
    get position(): Position;
    /** Sets position from a list or `{ x, y, z }`, writing all three values. */
    set position(value: Triple | Vec3Like);
    /** The window's placement: `position`, `rotation` and one uniform scale (the first component of `scale`). */
    get transform(): Transform3D;
    /**
     * Places the window: `{ position, rotation, scale }` or a `Transform3D`; a part left out stays as it is. The
     * parts are checked together, then written as `x`, `y`, `z`, `rotation` and `scale`, which go out in the frame's
     * one transaction (and, inside `set` or `requestTransition`, as one transition). `position.x` and `.y` are the
     * window's own rectangle, so a layout that places the window will place it again.
     */
    set transform(value: Transform3D | TransformInit);
    /**
     * The size the compositor last published for the window: what the client committed, as against `width` and
     * `height`, which are what the shell asked for (a write is not published until the client answers it).
     */
    get committedSize(): {
        width: number;
        height: number;
    };
    /**
     * The window this one is a dialog of (`xdg_toplevel.set_parent`), or null. A dialog is centred in its parent when
     * it opens, moves with it, stays on its workspace and output, and is left out of the layout's tiling.
     */
    get parentWindow(): Client | null;
    /**
     * Whether the window opened on an activation token the session's launcher minted, so `focus.newWindows` treats
     * it as launched. The compositor sets it when the window redeems such a token; a program started with
     * `gnoblin.spawn` is not marked, because the shell's spawner mints no token.
     */
    get fromLauncher(): boolean;
    /** Attaches a listener to this client; adding the same listener twice has no effect. */
    addListener(listener: ClientListener): void;
    /** Detaches a listener from this client. */
    removeListener(listener: ClientListener): void;
    /** Asks the client to close. The client is `alive` until the compositor destroys the window. */
    close(): void;
    /** Gives the window the keyboard focus: a write of the seat's `focus`, which is how a shell hands it out. */
    focus(): void;
    /**
     * The reactive properties of the window: `client.properties.bind("opacity", () => ...)` sets `opacity` now and
     * again whenever a property the function read changes. See `gnoblin-core/properties`.
     */
    get properties(): PropertyBindings;
    /** Shorthand for `client.properties.bind`. */
    bind(property: string, compute: () => unknown): PropertyBinding;
    /** Puts the window on top of every other window. */
    raise(): void;
    /** Puts the window under every other window. */
    lower(): void;
    /**
     * Kills the process that owns the window with SIGKILL: the not-responding alert's Force quit. The compositor
     * refuses, and does nothing, when the owner is unknown, has gone, changed since the window mapped, or is the
     * compositor itself or init.
     */
    kill(): void;
    /** Returns the compositor identifier in `Client#id` form. */
    toString(): string;
}
export interface Client extends Subscriptions<ClientListener> {
}
