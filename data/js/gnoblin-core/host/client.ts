// Live wrappers for compositor-managed toplevel windows.

import { GnoblinError } from "gnoblin-core/errors";
import { CREATE, host } from "gnoblin-core/host/bridge";
import { ctx } from "gnoblin-core/host/context";
import { hooks } from "gnoblin-core/host/hooks";
import { assertListener } from "gnoblin-core/host/listeners";
import type { ClientListener } from "gnoblin-core/host/listeners";
import { callMethod, publishedValues, read, refuse, stateOf, writeProperty } from "gnoblin-core/host/node_state";
import { NativeObject } from "gnoblin-core/native_object";
import { noteHostRead, propertiesOf } from "gnoblin-core/properties";
import type { PropertyBinding, PropertyBindings } from "gnoblin-core/properties";
import { Transform3D } from "gnoblin-core/math/transform";
import { Vector3 as Position } from "gnoblin-core/math/vector";
import { transformFrom, vectorFrom, withAxes } from "gnoblin-core/math/spatial";
import type { AxisTriple, ScaleInit, TransformInit, Triple, Vec3Like } from "gnoblin-core/math/spatial";
import type { NodeId, Rect } from "gnoblin-core/types";

/** How the compositor answers a window's frame callbacks: see `Client.framePolicy`. */
export type FramePolicy = "auto" | "hold" | "throttle";

/** The host properties frozen when a client leaves, so the object keeps its final readable state. */
export const CLIENT_KEYS = [
    "title",
    "app_id",
    "focused",
    "fullscreen",
    "maximised",
    "minimised",
    "x",
    "y",
    "width",
    "height",
    "tiled",
    "visible",
    "opacity",
    "z",
    "rotation",
    "scale",
    "secure",
    "parent_window",
    "from_launcher",
    "decoration",
    "decoration_wanted",
    "frame_policy",
    "frame_rate",
    "report_presented",
    "presented",
];

/** A toplevel window the compositor manages. */
export class Client extends NativeObject {
    constructor(token: symbol, node: NodeId) {
        super();
        if (token !== CREATE) {
            throw new TypeError("a Client is made by the compositor, not by a script");
        }
        ctx.states.set(this, {
            node,
            kind: "client",
            alive: true,
            frozen: null,
            listeners: [],
            readied: false,
            seen: publishedValues(node),
        });
    }

    /** Stable compositor identifier for this window. */
    get id(): string {
        return host.label(stateOf(this).node) ?? "?";
    }

    /** Whether the compositor still manages this window. */
    get alive(): boolean {
        return stateOf(this).alive;
    }

    /** Application identifier reported for the window. */
    get appId(): string {
        return read<string>(this, "app_id");
    }

    /** Current window title. */
    get title(): string {
        return read<string>(this, "title");
    }

    /** Whether this window has keyboard focus. */
    get focused(): boolean {
        return read<boolean>(this, "focused");
    }

    /** Whether the window covers its output. Writing asks the compositor to make it fullscreen or restore it. */
    get fullscreen(): boolean {
        return read<boolean>(this, "fullscreen");
    }

    /** Requests fullscreen or restores the window. */
    set fullscreen(value: boolean) {
        writeProperty(this, "fullscreen", value, "flag");
    }

    /** Whether the window is kept out of screen captures. */
    get secure(): boolean {
        return read<boolean>(this, "secure");
    }

    /** Controls whether the window is excluded from screen captures. */
    set secure(value: boolean) {
        writeProperty(this, "secure", value, "flag");
    }

    /** Whether the client has left a liveness ping unanswered too long. The compositor writes it, and clears it on an
     * answer.
     */
    get unresponsive(): boolean {
        return read<boolean>(this, "unresponsive");
    }

    /**
     * Who draws the window's frame, as the compositor negotiated it: `server` when the shell's titlebar does (the
     * client asked for server-side and this shell declared decorations, or the window is an X11 one), `client` when
     * the client draws its own title bar (a GTK, Qt or foot window), `none` for a window nobody frames. The
     * compositor writes it.
     * `frameInset` is taken only for a `server` window.
     */
    get decoration(): string {
        return read<string>(this, "decoration");
    }

    /** What the client asked of `xdg-decoration`: `client`, `server`, or `unset` (it asked nothing, or is an X11
     * window).
     */
    get decorationWanted(): string {
        return read<string>(this, "decoration_wanted");
    }

    /**
     * How the compositor answers this window's client when it asks to be told it may draw its next frame: `auto` after
     * every frame the compositor draws that shows the window, `hold` never (the client waits and draws nothing new,
     * which is what a shell wants for a window it does not show), `throttle` at most `frameRate` times a second. A held
     * or throttled window keeps its last picture and is drawn as before. It takes effect only while a shell holds the
     * tree, and the compositor sets every window back to `auto` when the shell lets go of it.
     */
    get framePolicy(): FramePolicy {
        return read<FramePolicy>(this, "frame_policy");
    }

    /** Sets how the window's client is answered; any other text than `auto`, `hold` or `throttle` is refused. */
    set framePolicy(value: FramePolicy) {
        writeProperty(this, "frame_policy", value, "text");
    }

    /**
     * The most frame callbacks a second a `throttle` window's client is answered, in hertz: above 0 and at most 1000.
     * It is read only while `framePolicy` is `throttle`.
     */
    get frameRate(): number {
        return read<number>(this, "frame_rate");
    }

    /** Sets the rate a `throttle` window's client is answered at, in hertz. */
    set frameRate(value: number) {
        writeProperty(this, "frame_rate", value, "number");
    }

    /**
     * Whether the compositor tells the shell when the window's pixels reach the screen: while it is true each frame
     * that draws the window writes `presented`. It is false by default, because a window written every frame is a
     * change in every batch.
     */
    get reportPresented(): boolean {
        return read<boolean>(this, "report_presented");
    }

    /** Asks the compositor to report, or stop reporting, when the window's pixels reach the screen. */
    set reportPresented(value: boolean) {
        writeProperty(this, "report_presented", value, "flag");
    }

    /**
     * When the last frame that showed the window reached the screen, in milliseconds on the compositor's monotonic
     * clock (`CLOCK_MONOTONIC`, which `wp_presentation` names), or 0 until a frame was presented with
     * `reportPresented` true. The compositor writes it.
     */
    get presented(): number {
        return read<number>(this, "presented");
    }

    /** Horizontal position in logical pixels. */
    get x(): number {
        return read<number>(this, "x");
    }

    /** Sets the horizontal position in logical pixels. */
    set x(value: number) {
        writeProperty(this, "x", value, "number");
    }

    /** Vertical position in logical pixels. */
    get y(): number {
        return read<number>(this, "y");
    }

    /** Sets the vertical position in logical pixels. */
    set y(value: number) {
        writeProperty(this, "y", value, "number");
    }

    /** Width in logical pixels. */
    get width(): number {
        return read<number>(this, "width");
    }

    /** Sets the width in logical pixels. */
    set width(value: number) {
        writeProperty(this, "width", value, "number");
    }

    /** Height in logical pixels. */
    get height(): number {
        return read<number>(this, "height");
    }

    /** Sets the height in logical pixels. */
    set height(value: number) {
        writeProperty(this, "height", value, "number");
    }

    /** The layout geometry. Assigning writes all four values, or none when one is not a finite number. */
    get bounds(): Rect {
        return { x: this.x, y: this.y, width: this.width, height: this.height };
    }

    /** Sets the content rectangle; all four values must be finite numbers. */
    set bounds(rect: Rect) {
        const target = `Client#${host.label(stateOf(this).node)}.bounds`;
        for (const key of ["x", "y", "width", "height"] as const) {
            if (typeof rect?.[key] !== "number" || !Number.isFinite(rect[key])) {
                throw new TypeError(`${target}.${key}: must be a finite number`);
            }
        }
        this.x = rect.x;
        this.y = rect.y;
        this.width = rect.width;
        this.height = rect.height;
    }

    /** Whether the window is maximised. Writing asks the compositor to maximise or restore it. */
    get maximised(): boolean {
        return read<boolean>(this, "maximised");
    }

    /** Requests maximisation or restoration. */
    set maximised(value: boolean) {
        writeProperty(this, "maximised", value, "flag");
    }

    /** Whether the window is minimised. Writing asks the compositor to minimise or restore it. */
    get minimised(): boolean {
        return read<boolean>(this, "minimised");
    }

    /** Requests minimisation or restoration. */
    set minimised(value: boolean) {
        writeProperty(this, "minimised", value, "flag");
    }

    /** Whether the window sits in a tiled layout, so its client drops the border and rounding it draws itself. */
    get tiled(): boolean {
        return read<boolean>(this, "tiled");
    }

    /** Sets whether the client is tiled. */
    set tiled(value: boolean) {
        writeProperty(this, "tiled", value, "flag");
    }

    /** Whether the window is drawn and hit. A workspace that is not shown hides its windows. */
    get visible(): boolean {
        return read<boolean>(this, "visible");
    }

    /** Sets whether the window is drawn and receives input. */
    set visible(value: boolean) {
        writeProperty(this, "visible", value, "flag");
    }

    /** How opaque the window is drawn, from 0 to 1 (the compositor clamps it). */
    get opacity(): number {
        return read<number>(this, "opacity");
    }

    /** Sets opacity from 0 (transparent) to 1 (opaque); the compositor clamps the value. */
    set opacity(value: number) {
        writeProperty(this, "opacity", value, "number");
    }

    /** The window's place in depth; negative is in front of the camera. */
    get z(): number {
        return read<number>(this, "z");
    }

    /** Sets the window's depth; negative values place it in front of the camera. */
    set z(value: number) {
        writeProperty(this, "z", value, "number");
    }

    /**
     * Rotation about each axis in radians; the list also answers to `x`, `y` and `z`. Assigning takes a list,
     * `{ x, y, z }` or a quaternion `{ x, y, z, w }`.
     */
    get rotation(): AxisTriple {
        return withAxes(read<never>(this, "rotation"));
    }

    /** Sets rotation about the x, y and z axes, in radians. */
    set rotation(value: Triple | Vec3Like) {
        writeProperty(this, "rotation", value, "vector");
    }

    /** Scale along each axis; the list also answers to `x`, `y` and `z`. Assigning takes one number for every axis,
     * a list or `{ x, y, z }`.
     */
    get scale(): AxisTriple {
        return withAxes(read<never>(this, "scale"));
    }

    /** Sets scale along the x, y and z axes. */
    set scale(value: ScaleInit) {
        writeProperty(this, "scale", value, "vector");
    }

    /** Where the window is: `x`, `y` and its depth `z`. Assigning takes a list or `{ x, y, z }` and writes the
     * three.
     */
    get position(): Position {
        return new Position(this.x, this.y, this.z);
    }

    /** Sets position from a list or `{ x, y, z }`, writing all three values. */
    set position(value: Triple | Vec3Like) {
        const [x, y, z] = vectorFrom(`${this}.position`, value);
        this.x = x;
        this.y = y;
        this.z = z;
    }

    /** The window's placement: `position`, `rotation` and one uniform scale (the first component of `scale`). */
    get transform(): Transform3D {
        return new Transform3D(this.position, this.rotation, this.scale[0]);
    }

    /**
     * Places the window: `{ position, rotation, scale }` or a `Transform3D`; a part left out stays as it is. The
     * parts are checked together, then written as `x`, `y`, `z`, `rotation` and `scale`, which go out in the frame's
     * one transaction (and, inside `set` or `requestTransition`, as one transition). `position.x` and `.y` are the
     * window's own rectangle, so a layout that places the window will place it again.
     */
    set transform(value: Transform3D | TransformInit) {
        const parts = transformFrom(`${this}.transform`, value);
        if (parts.position !== null) this.position = parts.position;
        if (parts.rotation !== null) this.rotation = parts.rotation;
        if (parts.scale !== null) this.scale = parts.scale;
    }

    /**
     * The size the compositor last published for the window: what the client committed, as against `width` and
     * `height`, which are what the shell asked for (a write is not published until the client answers it).
     */
    get committedSize(): { width: number; height: number } {
        const state = stateOf(this);
        if (state.frozen) {
            return { width: this.width, height: this.height };
        }
        const size = {
            width: host.published(state.node, "width") as number,
            height: host.published(state.node, "height") as number,
        };
        noteHostRead(state.node, "published:width", size.width);
        noteHostRead(state.node, "published:height", size.height);
        return size;
    }

    /**
     * The window this one is a dialog of (`xdg_toplevel.set_parent`), or null. A dialog is centred in its parent when
     * it opens, moves with it, stays on its workspace and output, and is left out of the layout's tiling.
     */
    get parentWindow(): Client | null {
        const node = read<number>(this, "parent_window");
        return node ? (ctx.clientsByNode.get(node) ?? null) : null;
    }

    /**
     * Whether the window opened on an activation token the session's launcher minted, so `focus.newWindows` treats
     * it as launched. The compositor sets it when the window redeems such a token; a program started with
     * `gnoblin.spawn` is not marked, because the shell's spawner mints no token.
     */
    get fromLauncher(): boolean {
        return read<boolean>(this, "from_launcher");
    }

    /** Attaches a listener to this client; adding the same listener twice has no effect. */
    addListener(listener: ClientListener): void {
        assertListener(listener, `Client#${host.label(stateOf(this).node)}.addListener`);
        const { listeners } = stateOf(this);
        if (!listeners.includes(listener)) {
            listeners.push(listener);
        }
    }

    /** Detaches a listener from this client. */
    removeListener(listener: ClientListener): void {
        const { listeners } = stateOf(this);
        const at = listeners.indexOf(listener);
        if (at >= 0) {
            listeners.splice(at, 1);
        }
    }

    /** Asks the client to close. The client is `alive` until the compositor destroys the window. */
    close(): void {
        callMethod(this, "close");
    }

    /** Gives the window the keyboard focus: a write of the seat's `focus`, which is how a shell hands it out. */
    focus(): void {
        callMethod(this, "focus");
    }

    /**
     * The reactive properties of the window: `client.properties.bind("opacity", () => ...)` sets `opacity` now and
     * again whenever a property the function read changes. See `gnoblin-core/properties`.
     */
    get properties(): PropertyBindings {
        return propertiesOf(this, () => `${this}`);
    }

    /** Shorthand for `client.properties.bind`. */
    bind(property: string, compute: () => unknown): PropertyBinding {
        return this.properties.bind(property, compute);
    }

    /** Puts the window on top of every other window. */
    raise(): void {
        const state = stateOf(this);
        const target = `Client#${host.label(state.node)}.raise()`;
        if (!state.alive) {
            throw new GnoblinError("not_found", target, "the client is gone");
        }
        const code = host.raise(state.node);
        if (code != null) {
            refuse(code, target, "client");
        }
        // A fullscreen sibling keeps the top: raising puts each fullscreen window back over the window it raised.
        if (!this.fullscreen) {
            for (const node of host.windows()) {
                const other = ctx.clientsByNode.get(node);
                if (other !== undefined && other !== this && other.fullscreen && hooks.sharesStack(this, other)) {
                    host.raise(node);
                }
            }
        }
    }

    /** Puts the window under every other window. */
    lower(): void {
        const state = stateOf(this);
        const target = `Client#${host.label(state.node)}.lower()`;
        if (!state.alive) {
            throw new GnoblinError("not_found", target, "the client is gone");
        }
        const code = host.lower(state.node);
        if (code != null) {
            refuse(code, target, "client");
        }
    }

    /**
     * Kills the process that owns the window with SIGKILL: the not-responding alert's Force quit. The compositor
     * refuses, and does nothing, when the owner is unknown, has gone, changed since the window mapped, or is the
     * compositor itself or init.
     */
    kill(): void {
        callMethod(this, "kill");
    }

    /** Returns the compositor identifier in `Client#id` form. */
    toString(): string {
        return `Client#${host.label(stateOf(this).node)}`;
    }
}
