// `Client`: a toplevel window the compositor manages. Every property reads the host mirror (or the frozen copy once
// the window is gone) and every write is queued as one tree write.

import { GnoblinError } from "gnoblin/errors";
import { CREATE, host } from "gnoblin/host/bridge";
import { ctx } from "gnoblin/host/context";
import { deskOfClient } from "gnoblin/host/desks";
import { assertListener } from "gnoblin/host/listeners";
import type { ClientListener } from "gnoblin/host/listeners";
import { moveClientTo } from "gnoblin/host/membership";
import { callMethod, isOutput, publishedValues, read, refuse, stateOf, writeProperty } from "gnoblin/host/node_state";
import type { Output } from "gnoblin/host/output";
import { motion } from "gnoblin/host/seat";
import { clientStyle, computedStyleOf, styleWarningsOf } from "gnoblin/host/style_scopes";
import type { Workspace } from "gnoblin/host/workspace";
import { wsOf } from "gnoblin/host/workspace_state";
import { NativeObject } from "gnoblin/native_object";
import { noteHostRead, propertiesOf } from "gnoblin/properties";
import type { PropertyBinding, PropertyBindings } from "gnoblin/properties";
import { repositioning } from "gnoblin/reposition";
import type { RepositionClient } from "gnoblin/reposition";
import { Transform3D } from "gnoblin/scene/transform";
import { Vector3 as Position } from "gnoblin/scene/vector";
import { transformFrom, vectorFrom, withAxes } from "gnoblin/spatial";
import type { AxisTriple, ScaleInit, TransformInit, Triple, Vec3Like } from "gnoblin/spatial";
import type { ComputedStyle } from "gnoblin/style/resolve";
import type { ClientStyle } from "gnoblin/style/style";
import { curveSamples, DEFAULT_EASING, EASING_NAMES, easingCurve } from "gnoblin/transition/easing";
import { setProperties } from "gnoblin/transition/engine";
import type { Easing } from "gnoblin/transition/easing";
import type { Transition } from "gnoblin/transition/transition";
import type { TransitionOptions } from "gnoblin/transition/types";
import type { NodeId, Rect } from "gnoblin/types";

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
];

/** The properties of a client `animate` can move, and the kind of value each takes. */
export const ANIMATED_KEYS: Readonly<Record<string, "number" | "vector">> = {
    x: "number",
    y: "number",
    width: "number",
    height: "number",
    opacity: "number",
    z: "number",
    rotation: "vector",
    scale: "vector",
};

/** What `Client.animate` takes: the values to move to, each of the kind its property holds. */
export interface ClientAnimation {
    x?: number;
    y?: number;
    width?: number;
    height?: number;
    opacity?: number;
    z?: number;
    rotation?: Triple;
    scale?: Triple;
}

/** Duration and easing options for a client animation. */
export interface AnimateOptions {
    /** Milliseconds. Zero is an instant write. */
    duration: number;
    /** One of the CSS names of `Easing`. Default `"ease-out"`. */
    easing?: Easing;
}

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
            floating: false,
        });
    }

    get id(): string {
        return host.label(stateOf(this).node) ?? "?";
    }

    get alive(): boolean {
        return stateOf(this).alive;
    }

    get appId(): string {
        return read<string>(this, "app_id");
    }

    get title(): string {
        return read<string>(this, "title");
    }

    get focused(): boolean {
        return read<boolean>(this, "focused");
    }

    /** Whether the window covers its output. Writing asks the compositor to make it fullscreen or restore it. */
    get fullscreen(): boolean {
        return read<boolean>(this, "fullscreen");
    }

    set fullscreen(value: boolean) {
        writeProperty(this, "fullscreen", value, "flag");
    }

    /** Whether the window is kept out of screen captures. */
    get secure(): boolean {
        return read<boolean>(this, "secure");
    }

    set secure(value: boolean) {
        writeProperty(this, "secure", value, "flag");
    }

    /** Whether the client has left a liveness ping unanswered too long. The compositor writes it, and clears it on an answer. */
    get unresponsive(): boolean {
        return read<boolean>(this, "unresponsive");
    }

    /**
     * Who draws the window's frame, as the compositor negotiated it: `server` when the shell's titlebar does (the client
     * asked for server-side and this shell declared decorations, or the window is an X11 one), `client` when the client
     * draws its own title bar (a GTK, Qt or foot window), `none` for a window nobody frames. The compositor writes it.
     * `frameInset` is reserved only for a `server` window.
     */
    get decoration(): string {
        return read<string>(this, "decoration");
    }

    /** What the client asked of `xdg-decoration`: `client`, `server`, or `unset` (it asked nothing, or is an X11 window). */
    get decorationWanted(): string {
        return read<string>(this, "decoration_wanted");
    }

    /**
     * The overrides this client has, and only those: a property that was never set here is `undefined` and comes from
     * the scopes above (`gnoblin.clients.style`, the workspace's, a `where()` selection's). Reading a list this client
     * does not override gives a copy of the inherited one; the first write to the copy makes it this client's own.
     */
    get style(): ClientStyle {
        return clientStyle(this);
    }

    /** The style this client resolves to now: frozen, with the layer each value came from in `origin`. */
    get computedStyle(): ComputedStyle {
        return computedStyleOf(this);
    }

    /** What of the client's style is not drawn and why, and the effects the compositor skipped. */
    get styleWarnings(): readonly string[] {
        return styleWarningsOf(this);
    }

    /**
     * The room a titlebar takes above the window's content, in pixels: 0 for a window with no titlebar or a fullscreen one.
     * `bounds` is the content's rectangle; the frame a layout places is the content with this much added on top (see
     * docs/js-ui.md, "Titlebar").
     */
    get frameInset(): number {
        return ctx.decorations?.insetOf(this) ?? 0;
    }

    /**
     * Whether the window floats on a workspace whose layout tiles: a floating client is not given to the layout's
     * `compute`, so it keeps the place it has. Only the script knows this; the compositor does not.
     */
    get floating(): boolean {
        return stateOf(this).floating;
    }

    set floating(value: boolean) {
        if (typeof value !== "boolean") {
            throw new TypeError(`Client#${host.label(stateOf(this).node)}.floating: must be true or false`);
        }
        const state = stateOf(this);
        if (state.floating !== value) {
            state.floating = value;
            const own = wsOf(this);
            if (own) {
                own.dirty = true;
            }
        }
    }

    get x(): number {
        return read<number>(this, "x");
    }

    set x(value: number) {
        writeProperty(this, "x", value, "number");
    }

    get y(): number {
        return read<number>(this, "y");
    }

    set y(value: number) {
        writeProperty(this, "y", value, "number");
    }

    get width(): number {
        return read<number>(this, "width");
    }

    set width(value: number) {
        writeProperty(this, "width", value, "number");
    }

    get height(): number {
        return read<number>(this, "height");
    }

    set height(value: number) {
        writeProperty(this, "height", value, "number");
    }

    /** The layout geometry. Assigning writes all four values, or none when one is not a finite number. */
    get bounds(): Rect {
        return { x: this.x, y: this.y, width: this.width, height: this.height };
    }

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

    set maximised(value: boolean) {
        writeProperty(this, "maximised", value, "flag");
    }

    /** Whether the window is minimised. Writing asks the compositor to minimise or restore it. */
    get minimised(): boolean {
        return read<boolean>(this, "minimised");
    }

    set minimised(value: boolean) {
        writeProperty(this, "minimised", value, "flag");
    }

    /** Whether the window sits in a tiled layout, so its client drops the border and rounding it draws itself. */
    get tiled(): boolean {
        return read<boolean>(this, "tiled");
    }

    set tiled(value: boolean) {
        writeProperty(this, "tiled", value, "flag");
    }

    /** Whether the window is drawn and hit. A workspace that is not shown hides its windows. */
    get visible(): boolean {
        return read<boolean>(this, "visible");
    }

    set visible(value: boolean) {
        writeProperty(this, "visible", value, "flag");
    }

    /** How opaque the window is drawn, from 0 to 1 (the compositor clamps it). */
    get opacity(): number {
        return read<number>(this, "opacity");
    }

    set opacity(value: number) {
        writeProperty(this, "opacity", value, "number");
    }

    /** The window's place in depth; negative is in front of the camera. */
    get z(): number {
        return read<number>(this, "z");
    }

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

    set rotation(value: Triple | Vec3Like) {
        writeProperty(this, "rotation", value, "vector");
    }

    /** Scale along each axis; the list also answers to `x`, `y` and `z`. Assigning takes one number for every axis, a list or `{ x, y, z }`. */
    get scale(): AxisTriple {
        return withAxes(read<never>(this, "scale"));
    }

    set scale(value: ScaleInit) {
        writeProperty(this, "scale", value, "vector");
    }

    /** Where the window is: `x`, `y` and its depth `z`. Assigning takes a list or `{ x, y, z }` and writes the three. */
    get position(): Position {
        return new Position(this.x, this.y, this.z);
    }

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
     * parts are checked together, then written as `x`, `y`, `z`, `rotation` and `scale`, which go out in the frame's one
     * transaction (and, inside `set` or `requestTransition`, as one transition). `position.x` and `.y` are the
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

    /** The workspace the client is on, or null. */
    get workspace(): Workspace | null {
        return ctx.membership.get(this) ?? null;
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
     * Whether the window opened on an activation token the session's launcher minted, so `focus.newWindows` treats it as
     * launched. The compositor sets it when the window redeems such a token; a program started with `gnoblin.spawn` is
     * not marked, because the shell's spawner mints no token.
     */
    get fromLauncher(): boolean {
        return read<boolean>(this, "from_launcher");
    }

    /** The output that shows the client's workspace, or null while no output shows it (or the output is gone). */
    get output(): Output | null {
        const desk = deskOfClient(this);
        return desk?.parked === false ? desk.output : null;
    }

    /**
     * Moves the window to `workspace` and fits it to that workspace's work area: a maximised window fills it, a snapped
     * one re-snaps, a floating one is shrunk to fit and moved inside it. A window that leaves the shown workspace hands
     * its focus to a window that stays. Its dialogs go with it. Refused (false, and nothing moves) for a fullscreen
     * window, a dialog (it moves only with its parent), a dead window, a workspace that is not in
     * `gnoblin.workspaces`, or the workspace it is on already.
     */
    moveToWorkspace(workspace: Workspace): boolean {
        return moveClientTo(this, workspace);
    }

    /** `moveToWorkspace` to the workspace `output` shows. False when the output is gone or shows none. */
    moveToOutput(output: Output): boolean {
        if (!isOutput(output)) {
            throw new TypeError(`${this}.moveToOutput: needs an Output`);
        }
        const shown = output.alive ? output.workspace : null;
        return shown !== null && moveClientTo(this, shown);
    }

    addListener(listener: ClientListener): void {
        assertListener(listener, `Client#${host.label(stateOf(this).node)}.addListener`);
        const { listeners } = stateOf(this);
        if (!listeners.includes(listener)) {
            listeners.push(listener);
        }
    }

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
     * Starts moving the window with the arrow keys, 16 pixels a press, inside the work area: Return keeps the place,
     * Escape puts the window back. Refused (false) for a window that is fullscreen, maximised, tiled or not showing; a
     * snapped window is unsnapped first. See `gnoblin/reposition`.
     */
    moveWithKeys(): boolean {
        return repositioning.begin(this as unknown as RepositionClient, "move");
    }

    /** Starts resizing the window with the arrow keys: Right and Down grow it, Left and Up shrink it. */
    resizeWithKeys(): boolean {
        return repositioning.begin(this as unknown as RepositionClient, "resize");
    }

    /**
     * Moves properties to `values` over `options.duration` milliseconds along the easing curve. Reading a property back
     * at once gives the end value; the compositor draws the way there. A duration of 0 writes at once.
     */
    animate(values: ClientAnimation, options: AnimateOptions): void {
        const state = stateOf(this);
        const target = `Client#${host.label(state.node)}.animate()`;
        if (!state.alive) {
            throw new GnoblinError("not_found", target, "the client is gone");
        }
        const duration = options?.duration;
        if (typeof duration !== "number" || !Number.isFinite(duration) || duration < 0) {
            throw new TypeError(`${target}: duration must be a finite number of milliseconds of at least 0`);
        }
        const curve = easingCurve(options.easing ?? DEFAULT_EASING);
        if (curve === undefined) {
            throw new TypeError(`${target}: easing must be one of ${EASING_NAMES}`);
        }
        const entries = Object.entries(values).sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0));
        for (const [key, value] of entries) {
            const kind = ANIMATED_KEYS[key];
            if (kind === undefined) {
                throw new TypeError(`${target}: ${key} cannot be animated`);
            }
            if (kind === "number") {
                if (typeof value !== "number" || !Number.isFinite(value)) {
                    throw new TypeError(`${target}: ${key} must be a finite number`);
                }
            } else if (
                !Array.isArray(value) ||
                value.length !== 3 ||
                value.some((n) => typeof n !== "number" || !Number.isFinite(n))
            ) {
                throw new TypeError(`${target}: ${key} must be three finite numbers`);
            }
        }
        const samples = curveSamples(curve);
        // Reduced motion lands the value at once, as the compositor's own safety net would.
        const effective = motion(duration);
        for (const [key, value] of entries) {
            const sent = Array.isArray(value) ? [value[0], value[1], value[2]] : (value as number);
            const code =
                effective === 0
                    ? host.set(state.node, key, sent)
                    : host.animate(state.node, key, sent, effective, samples);
            if (code != null) {
                refuse(code, target, "client");
            }
        }
    }

    /**
     * Assigns `props` as one `requestTransition`: the same as `gnoblin.requestTransition(() => { client.opacity = ... })`
     * for this client. Without `options` the change is atomic and immediate. `options.duration` is in milliseconds.
     */
    set(props: Record<string, unknown>, options?: TransitionOptions): Transition {
        return setProperties(ctx.transitions, this, `${this}`, props, options);
    }

    /**
     * The reactive properties of the window: `client.properties.bind("opacity", () => ...)` sets `opacity` now and
     * again whenever a property the function read changes. See `gnoblin/properties`.
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
        // A fullscreen sibling keeps the top: the the retired shell `Window:raise` put each back over the window it raised.
        if (!this.fullscreen) {
            const own = ctx.membership.get(this) ?? null;
            for (const node of host.windows()) {
                const other = ctx.clientsByNode.get(node);
                if (
                    other !== undefined &&
                    other !== this &&
                    (ctx.membership.get(other) ?? null) === own &&
                    other.fullscreen
                ) {
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
     * Kills the process that owns the window with SIGKILL: the not-responding alert's Force quit. The compositor refuses,
     * and does nothing, when the owner is unknown, has gone, changed since the window mapped, or is the compositor itself
     * or init.
     */
    kill(): void {
        callMethod(this, "kill");
    }

    toString(): string {
        return `Client#${host.label(stateOf(this).node)}`;
    }
}
