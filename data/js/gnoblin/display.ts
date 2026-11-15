// Logical displays (owner-spec section 9, cameras in section 8): `OutputProfile`, `Display`, `Viewport` and
// `Camera2D`. This module owns the objects a script holds and their checked state. What a display does to the
// hardware and to the desks (which workspace an output shows, a mirror, a span) is `DisplayManager`
// (`gnoblin/display_manager`), which reads the state kept here through the `*State` accessors below.
//
// The state of each object is kept in a WeakMap, not in `#` fields, because the manager sits in another module.

import { Camera3D, Node } from "gnoblin/scene";
import type { NodeOptions } from "gnoblin/scene";
import type { ErrorCode } from "gnoblin/errors";

/** The transforms an output takes: the compositor's names for a rotation and a flip. */
export type OutputTransform =
    "normal" | "90" | "180" | "270" | "flipped" | "flipped-90" | "flipped-180" | "flipped-270";

/**
 * What an output is told to do, as the argument of `Output.apply` and the settings of an `OutputProfile`. A key left
 * out keeps the output's value. The compositor applies it through its transactional engine and keeps it as the
 * output's profile, so the same monitor on the same port gets it back after an unplug. `primary` is not a setting: the
 * layout derives the primary output, and a request that names it throws.
 */
export interface OutputConfig {
    /** `"3440x1440@144"`, or `"preferred"` to keep what the backend drives. */
    mode?: string;
    /** From 0.5 to 4. */
    scale?: number;
    /**
     * Owner-requested addition beyond the display specification. The output is composed at this multiple of its mode
     * (`mode * supersample` pixels, clients told `scale * supersample`) and shown at its mode, so the desktop keeps its
     * logical size and looks anti-aliased, and screenshots come out hi-res. A number from 1 to 16; 1, 2 and 4 are the
     * supported ones, any other is accepted when `mode * supersample` is a whole size, with one notice.
     */
    supersample?: number;
    transform?: OutputTransform;
    /** The output's top-left corner in the one desktop coordinate space, `[x, y]`. Left out, it follows the row. */
    position?: readonly [number, number] | { readonly x: number; readonly y: number };
    /** `false` switches the output off: it shows black and has no place in the desktop. */
    enabled?: boolean;
    /** The connector this output shows an exact copy of, letterboxed to fit its own panel. */
    mirrorOf?: string;
}

/** The keys `outputRequest` accepts, for the error text of an unknown one. */
const OUTPUT_KEYS: readonly string[] = ["mode", "scale", "supersample", "transform", "position", "enabled", "mirrorOf"];

/** The scale range the compositor accepts for an output. */
const MIN_OUTPUT_SCALE = 0.5;
const MAX_OUTPUT_SCALE = 4;

/** The supersample range the compositor accepts for an output. */
const MIN_SUPERSAMPLE = 1;
const MAX_SUPERSAMPLE = 16;

/** The same names as `OutputTransform`, as a list, to check a value at run time. */
const TRANSFORMS: readonly string[] = [
    "normal",
    "90",
    "180",
    "270",
    "flipped",
    "flipped-90",
    "flipped-180",
    "flipped-270",
];

/**
 * The compositor's request text for `setting` (one `key=value` per line), checking each value and naming `path` and the
 * key in the error. `mirrorOf` is already a connector here; `Output.mirror` turns an `Output` into one.
 */
export function outputRequest(
    setting: Omit<OutputConfig, "mirrorOf"> & { mirrorOf?: string | null },
    path: string,
): string {
    if (setting === null || typeof setting !== "object") {
        throw new TypeError(`${path} must be an object with ${OUTPUT_KEYS.join(", ")}`);
    }
    const lines: string[] = [];
    for (const key of Object.keys(setting)) {
        const value = (setting as Record<string, unknown>)[key];
        const where = `${path}.${key}`;
        switch (key) {
            case "mode":
                if (typeof value !== "string" || !(value === "preferred" || /^\d+x\d+@\d+(\.\d+)?$/.test(value))) {
                    throw new TypeError(
                        `${where} must be "preferred" or "WIDTHxHEIGHT@HERTZ", such as "3440x1440@144"`,
                    );
                }
                lines.push(`mode=${value}`);
                break;
            case "scale":
                if (
                    typeof value !== "number" ||
                    !Number.isFinite(value) ||
                    value < MIN_OUTPUT_SCALE ||
                    value > MAX_OUTPUT_SCALE
                ) {
                    throw new RangeError(`${where} must be a number from ${MIN_OUTPUT_SCALE} to ${MAX_OUTPUT_SCALE}`);
                }
                lines.push(`scale=${value}`);
                break;
            case "supersample":
                if (
                    typeof value !== "number" ||
                    !Number.isFinite(value) ||
                    value < MIN_SUPERSAMPLE ||
                    value > MAX_SUPERSAMPLE
                ) {
                    throw new RangeError(
                        `${where} must be a number from ${MIN_SUPERSAMPLE} to ${MAX_SUPERSAMPLE}, such as 1, 2 or 4`,
                    );
                }
                lines.push(`supersample=${value}`);
                break;
            case "transform":
                if (typeof value !== "string" || !TRANSFORMS.includes(value)) {
                    throw new TypeError(`${where} must be one of ${TRANSFORMS.join(", ")}`);
                }
                lines.push(`transform=${value}`);
                break;
            case "position": {
                const [x, y] = Array.isArray(value)
                    ? (value as unknown[])
                    : value !== null && typeof value === "object"
                      ? [(value as { x?: unknown }).x, (value as { y?: unknown }).y]
                      : [];
                if (!Number.isInteger(x) || !Number.isInteger(y)) {
                    throw new TypeError(`${where} must be [x, y] or { x, y } with whole numbers`);
                }
                lines.push(`position=${String(x)},${String(y)}`);
                break;
            }
            case "enabled":
                if (typeof value !== "boolean") {
                    throw new TypeError(`${where} must be true or false`);
                }
                lines.push(`enabled=${String(value)}`);
                break;
            case "mirrorOf":
                if (value !== null && (typeof value !== "string" || value === "")) {
                    throw new TypeError(`${where} must be a connector name such as "eDP-1", or null for no mirror`);
                }
                lines.push(`mirror_of=${value ?? ""}`);
                break;
            case "primary":
                // Refused by name instead of left to the generic message: it is an output property, and a script that
                // wrote it would otherwise read the "unknown key" text as a typo.
                throw new TypeError(`${where} cannot be set: the layout derives the primary output`);
            default:
                throw new TypeError(`${where} is not an output setting; expected one of ${OUTPUT_KEYS.join(", ")}`);
        }
    }
    return lines.join("\n");
}

/** What `outputRequest` takes: the keys of one output's request. */
export type OutputSetting = Parameters<typeof outputRequest>[0];
/** The live output collection a display presents. */
export type DisplayOutputCollection = Iterable<object> & { readonly length: number };

/** A point in a plane, as the specification's `Vec2`. */
export interface DisplayVec2 {
    x: number;
    y: number;
}

/** Which hardware a profile is for. Every key given must match; at least one is required. */
export interface OutputMatch {
    /** The connector name (`DP-1`). */
    connector?: string;
    /** The manufacturer as the display reports it. */
    make?: string;
    /** The model name as the display reports it. */
    model?: string;
    /** The serial number from the monitor's EDID identity. */
    serial?: string;
}

/** A mode as a profile writes it: `"preferred"`, `"WIDTHxHEIGHT@HERTZ"` or the parts. */
export type OutputProfileMode =
    "preferred" | string | { readonly width: number; readonly height: number; readonly refreshRate: number };

/** The edge of the neighbouring output a position is against, and where along that edge the output sits. */
export type OutputAlign = "top" | "center" | "bottom" | "left" | "right";

/** Where a profile's output is placed: a pixel position, or against the output of another profile. */
export type OutputProfilePosition =
    | { readonly x: number; readonly y: number }
    | { readonly rightOf: PlacementTarget; readonly align?: OutputAlign }
    | { readonly leftOf: PlacementTarget; readonly align?: OutputAlign }
    | { readonly above: PlacementTarget; readonly align?: OutputAlign }
    | { readonly below: PlacementTarget; readonly align?: OutputAlign };

/** What a relative position names: the profile of the neighbour, or an `Output` itself. */
export type PlacementTarget = OutputProfile | OutputView;

/** What the display classes read of a connected `Output`. The shell's `Output` is one. */
export interface OutputView {
    readonly id: string;
    readonly name: string;
    readonly connector: string;
    readonly make: string;
    readonly model: string;
    readonly identity: string;
    readonly x: number;
    readonly y: number;
    readonly width: number;
    readonly height: number;
    readonly connected: boolean;
}

/** The arguments of `new OutputProfile`. */
export interface OutputProfileInit {
    /** Stable name of the profile; default: the `match` written out. */
    id?: string;
    match: OutputMatch;
    mode?: OutputProfileMode;
    /** From 0.5 to 4. */
    scale?: number;
    /** Composes the output at this multiple of its mode; see `OutputConfig.supersample`. */
    supersample?: number;
    transform?: OutputTransform;
    /** Asks for variable refresh rate. Accepted and reported as not applied: the backends have none. */
    vrr?: boolean;
    position?: OutputProfilePosition;
    /** `false` switches the output off. */
    enabled?: boolean;
}

/** The three ways a display presents its outputs. */
export type DisplayModeName = "single" | "mirror" | "span";

/** The arguments of `new Display`. */
export interface DisplayInit {
    /** Stable name of the display; default `display-N` in creation order. */
    id?: string;
    /** The outputs the display presents: profiles (persistent selectors) or connected outputs. */
    outputs: readonly (OutputProfile | OutputView)[];
    /** `single` (default) presents one output; `mirror` and `span` present several. */
    mode?: DisplayModeName;
    viewport?: Viewport;
}

/** The arguments of `new Viewport`. */
export interface ViewportInit {
    /** The workspace to show. */
    content?: object;
    /** Default: a `Camera2D` with no zoom, pan or rotation. */
    camera?: Camera2D | Camera3D;
    /** The colour behind the content. Kept and readable; no renderer draws it yet. */
    clearColor?: string;
}

/** The arguments of `new Camera2D`. */
export interface Camera2DInit {
    name?: string;
    /** Where the camera looks, in pixels of the content. Default `{ x: 0, y: 0 }`. */
    position?: DisplayVec2;
    /** Magnification: 1 shows the content at its size. Must be above 0. */
    zoom?: number;
    /** Radians, about the view axis. */
    rotation?: number;
}

/** The facts a camera or display hands the manager. */
export interface DisplayHooks {
    /** Something a profile, display or viewport holds changed: the manager must reconcile. */
    changed(): void;
    /** Whether `value` is a workspace of the shell. */
    isWorkspace(value: unknown): boolean;
    /** Whether `value` is a connected output of the shell. */
    isOutput(value: unknown): boolean;
    /** The error type the shell throws, named by `code`, `target` and `message`. */
    error(code: ErrorCode, target: string, message: string): Error;
    /** What the display's viewport shows now, or undefined when the display has no output to show it on. */
    shown(display: Display): object | null | undefined;
    /** Shows `workspace` on the display, or keeps it until the display has an output. Throws when it cannot be shown. */
    show(display: Display, workspace: object): void;
    /** The outputs a display presents now, as a `Collection<Output>`. */
    outputs(display: Display): DisplayOutputCollection;
}

let hooks: DisplayHooks | null = null;

/** Gives the module the shell's side. The host module calls it once per generation, before any user code runs. */
export function attachDisplays(next: DisplayHooks): void {
    hooks = next;
    counter = 0;
}

function shell(): DisplayHooks {
    if (hooks === null) {
        throw new Error("displays need the shell host and cannot run without it");
    }
    return hooks;
}

/** Counts the displays made without an `id`. */
let counter = 0;

function finite(target: string, value: unknown): number {
    if (typeof value !== "number" || !Number.isFinite(value)) {
        throw new TypeError(`${target}: must be a finite number`);
    }
    return value;
}

// ---- OutputProfile ------------------------------------------------------------------------------------------------

const MATCH_KEYS: readonly (keyof OutputMatch)[] = ["connector", "make", "model", "serial"];
const SIDES = ["rightOf", "leftOf", "above", "below"] as const;
const ALIGN_ALONG: Record<(typeof SIDES)[number], readonly OutputAlign[]> = {
    rightOf: ["top", "center", "bottom"],
    leftOf: ["top", "center", "bottom"],
    above: ["left", "center", "right"],
    below: ["left", "center", "right"],
};

/** The state of one profile. Not exported to scripts: the manager reads it. */
export interface ProfileState {
    id: string;
    match: Readonly<OutputMatch>;
    mode: OutputProfileMode | undefined;
    scale: number | undefined;
    supersample: number | undefined;
    transform: OutputTransform | undefined;
    vrr: boolean | undefined;
    position: OutputProfilePosition | undefined;
    enabled: boolean | undefined;
}

const profiles = new WeakMap<OutputProfile, ProfileState>();

/** The state of a profile, for the manager. */
export function profileState(profile: OutputProfile): ProfileState {
    const state = profiles.get(profile);
    if (state === undefined) {
        throw new TypeError("not an OutputProfile");
    }
    return state;
}

function describeMatch(match: OutputMatch): string {
    return MATCH_KEYS.filter((key) => match[key] !== undefined)
        .map((key) => String(match[key]))
        .join("/");
}

function checkMatch(match: unknown, where: string): Readonly<OutputMatch> {
    if (match === null || typeof match !== "object" || Array.isArray(match)) {
        throw new TypeError(`${where}.match: must be an object with ${MATCH_KEYS.join(", ")}`);
    }
    const checked: OutputMatch = {};
    for (const [key, value] of Object.entries(match)) {
        if (!(MATCH_KEYS as readonly string[]).includes(key)) {
            throw new TypeError(`${where}.match.${key}: not a match key; expected one of ${MATCH_KEYS.join(", ")}`);
        }
        if (typeof value !== "string" || value === "") {
            throw new TypeError(`${where}.match.${key}: must be a non-empty string`);
        }
        checked[key as keyof OutputMatch] = value;
    }
    if (Object.keys(checked).length === 0) {
        throw new TypeError(`${where}.match: needs at least one of ${MATCH_KEYS.join(", ")}`);
    }
    return Object.freeze(checked);
}

/** The text a mode is sent as: `preferred` or `WxH@R`. */
function modeText(mode: OutputProfileMode, where: string): string {
    if (typeof mode === "string") {
        return mode;
    }
    if (mode === null || typeof mode !== "object") {
        throw new TypeError(
            `${where}.mode: must be "preferred", "WIDTHxHEIGHT@HERTZ" or { width, height, refreshRate }`,
        );
    }
    const { width, height, refreshRate } = mode;
    for (const [key, value] of [
        ["width", width],
        ["height", height],
        ["refreshRate", refreshRate],
    ] as const) {
        if (typeof value !== "number" || !Number.isFinite(value) || value <= 0) {
            throw new TypeError(
                `${where}.mode.${key}: must be a number above 0 (a mode is { width, height, refreshRate })`,
            );
        }
    }
    if (!Number.isInteger(width) || !Number.isInteger(height)) {
        throw new TypeError(`${where}.mode: width and height must be whole numbers of pixels`);
    }
    return `${width}x${height}@${refreshRate}`;
}

function checkPosition(position: unknown, where: string): OutputProfilePosition {
    if (position === null || typeof position !== "object") {
        throw new TypeError(`${where}.position: must be { x, y } or { rightOf | leftOf | above | below, align }`);
    }
    const given = position as Record<string, unknown>;
    const side = SIDES.find((key) => key in given);
    if (side === undefined) {
        if (!Number.isInteger(given.x) || !Number.isInteger(given.y)) {
            throw new TypeError(
                `${where}.position: must be { x, y } with whole numbers, or name a side such as rightOf`,
            );
        }
        return Object.freeze({ x: given.x as number, y: given.y as number });
    }
    for (const key of Object.keys(given)) {
        if (key !== side && key !== "align") {
            throw new TypeError(`${where}.position.${key}: a relative position takes one side and align, not ${key}`);
        }
    }
    const target = given[side];
    if (!(target instanceof OutputProfile) && !shell().isOutput(target)) {
        throw new TypeError(`${where}.position.${side}: must be an OutputProfile or an Output`);
    }
    const align = given.align;
    if (align !== undefined && !ALIGN_ALONG[side].includes(align as OutputAlign)) {
        throw new TypeError(`${where}.position.align: along ${side} it must be one of ${ALIGN_ALONG[side].join(", ")}`);
    }
    return Object.freeze({ [side]: target, ...(align === undefined ? {} : { align }) }) as OutputProfilePosition;
}

/**
 * The persistent hardware settings for one monitor: the mode, scale, transform and place it should have, and which
 * monitor that is. A profile is inert while no matching output is connected, and applies when one is (also after a
 * hotplug). It is put in `gnoblin.outputs.profiles`; the compositor applies it as a transaction (an overlap, a gap or a
 * mode the backend refuses leaves the last good arrangement, and the refusal is reported).
 *
 * Properties other than `match` may be changed later; the change applies on the next frame. The compositor keeps the
 * last arrangement it applied after a profile is removed.
 */
export class OutputProfile {
    /**
     * @throws TypeError when a key is unknown or a value is out of range (the message names the key)
     */
    constructor(init: OutputProfileInit) {
        if (init === null || typeof init !== "object") {
            throw new TypeError("OutputProfile: needs an object with match and the settings");
        }
        const { id, match, mode, scale, supersample, transform, vrr, position, enabled, ...unknown } = init;
        const extra = Object.keys(unknown);
        if (extra.length > 0) {
            throw new TypeError(`OutputProfile.${extra[0]}: not a setting of a profile`);
        }
        const checked = checkMatch(match, "OutputProfile");
        const name = id ?? describeMatch(checked);
        if (typeof name !== "string" || name === "") {
            throw new TypeError("OutputProfile.id: must be a non-empty string");
        }
        profiles.set(this, {
            id: name,
            match: checked,
            mode: undefined,
            scale: undefined,
            supersample: undefined,
            transform: undefined,
            vrr: undefined,
            position: undefined,
            enabled: undefined,
        });
        if (mode !== undefined) this.mode = mode;
        if (scale !== undefined) this.scale = scale;
        if (supersample !== undefined) this.supersample = supersample;
        if (transform !== undefined) this.transform = transform;
        if (vrr !== undefined) this.vrr = vrr;
        if (position !== undefined) this.position = position;
        if (enabled !== undefined) this.enabled = enabled;
    }

    /** The profile's name in `gnoblin.outputs.profiles`. */
    get id(): string {
        return profileState(this).id;
    }

    /** Which monitor the profile is for. Fixed at construction. */
    get match(): Readonly<OutputMatch> {
        return profileState(this).match;
    }

    get mode(): OutputProfileMode | undefined {
        return profileState(this).mode;
    }
    set mode(value: OutputProfileMode) {
        this.#change("mode", value, { mode: modeText(value, `OutputProfile "${this.id}"`) });
    }

    get scale(): number | undefined {
        return profileState(this).scale;
    }
    set scale(value: number) {
        this.#change("scale", value, { scale: value });
    }

    get supersample(): number | undefined {
        return profileState(this).supersample;
    }
    set supersample(value: number) {
        this.#change("supersample", value, { supersample: value });
    }

    get transform(): OutputTransform | undefined {
        return profileState(this).transform;
    }
    set transform(value: OutputTransform) {
        this.#change("transform", value, { transform: value });
    }

    /** Whether variable refresh rate is asked for. Reported as not applied. */
    get vrr(): boolean | undefined {
        return profileState(this).vrr;
    }
    set vrr(value: boolean) {
        if (typeof value !== "boolean") {
            throw new TypeError(`OutputProfile "${this.id}".vrr: must be true or false`);
        }
        profileState(this).vrr = value;
        shell().changed();
    }

    get position(): OutputProfilePosition | undefined {
        return profileState(this).position;
    }
    set position(value: OutputProfilePosition) {
        const checked = checkPosition(value, `OutputProfile "${this.id}"`);
        const state = profileState(this);
        const before = state.position;
        state.position = checked;
        try {
            assertAcyclic(this);
        } catch (error) {
            state.position = before;
            throw error;
        }
        shell().changed();
    }

    get enabled(): boolean | undefined {
        return profileState(this).enabled;
    }
    set enabled(value: boolean) {
        this.#change("enabled", value, { enabled: value });
    }

    #change<K extends "mode" | "scale" | "supersample" | "transform" | "enabled">(
        key: K,
        value: ProfileState[K],
        check: OutputSetting,
    ): void {
        // The same checks `Output.apply` makes: a bad value throws here, naming `OutputProfile "id".key`.
        outputRequest(check, `OutputProfile "${this.id}"`);
        profileState(this)[key] = value;
        shell().changed();
    }

    toString(): string {
        return `OutputProfile#${this.id}`;
    }
}

/** Throws when following `position` references from `profile` comes back to it. */
function assertAcyclic(profile: OutputProfile): void {
    const seen = new Set<OutputProfile>([profile]);
    let at: OutputProfile | null = profile;
    while (at !== null) {
        const target: unknown = relativeTarget(profileState(at).position)?.target;
        if (!(target instanceof OutputProfile)) {
            return;
        }
        if (seen.has(target)) {
            throw new TypeError(
                `OutputProfile "${profile.id}".position: placed relative to itself through "${target.id}"`,
            );
        }
        seen.add(target);
        at = target;
    }
}

/** The side, target and alignment of a relative position, or null for a pixel position or none. */
export function relativeTarget(
    position: OutputProfilePosition | undefined,
): { side: (typeof SIDES)[number]; target: PlacementTarget; align: OutputAlign | undefined } | null {
    if (position === undefined) {
        return null;
    }
    const given = position as Record<string, unknown>;
    const side = SIDES.find((key) => key in given);
    if (side === undefined) {
        return null;
    }
    return { side, target: given[side] as PlacementTarget, align: given.align as OutputAlign | undefined };
}

/** The serial number in an EDID identity `MAN:product:serial`, or "". */
export function serialOf(identity: string): string {
    const parts = identity.split(":");
    return parts.length < 3 ? "" : parts.slice(2).join(":");
}

/** Whether `output` is the hardware `profile` is for. */
export function profileMatches(profile: OutputProfile, output: OutputView): boolean {
    const { match } = profileState(profile);
    return (
        (match.connector === undefined || output.connector === match.connector) &&
        (match.make === undefined || output.make === match.make) &&
        (match.model === undefined || output.model === match.model) &&
        (match.serial === undefined || serialOf(output.identity) === match.serial)
    );
}

/** The request lines a profile makes of its output, leaving out a relative position, which needs its neighbour. */
export function profileSetting(profile: OutputProfile): OutputSetting {
    const state = profileState(profile);
    const setting: Record<string, unknown> = {};
    if (state.mode !== undefined) setting.mode = modeText(state.mode, `OutputProfile "${state.id}"`);
    if (state.scale !== undefined) setting.scale = state.scale;
    if (state.supersample !== undefined) setting.supersample = state.supersample;
    if (state.transform !== undefined) setting.transform = state.transform;
    if (state.enabled !== undefined) setting.enabled = state.enabled;
    if (state.position !== undefined && relativeTarget(state.position) === null) setting.position = state.position;
    return setting as OutputSetting;
}

// ---- Camera2D -----------------------------------------------------------------------------------------------------

/**
 * A flat camera: where the view looks, how far it is magnified and its roll. It is a scene node of the tree, so a
 * `requestTransition` can animate it: `position` is the node's `x` and `y`, `zoom` its scale and `rotation` its turn
 * about the view axis. Assigned to `viewport.camera`, the shell reports the properties it cannot draw (see
 * design/js-spec-native.md, part 2, "Cameras").
 */
export class Camera2D extends Node {
    constructor(options: Camera2DInit = {}) {
        super(options as never);
    }

    protected override assign(options: NodeOptions): void {
        const { position, zoom, rotation, ...rest } = options as Camera2DInit;
        super.assign(rest);
        if (position !== undefined) this.position = position;
        if (zoom !== undefined) this.zoom = zoom;
        if (rotation !== undefined) this.rotation = rotation as never;
    }

    /** Where the camera looks, in pixels of the content. */
    // @ts-expect-error The specification narrows `Node.position` to a flat camera's x/y point.
    override get position(): DisplayVec2 {
        return { x: this.x, y: this.y };
    }
    // @ts-expect-error As the getter above.
    override set position(value: DisplayVec2) {
        if (value === null || typeof value !== "object") {
            throw new TypeError(`${this}.position: must be { x, y }`);
        }
        this.x = finite(`${this}.position.x`, value.x);
        this.y = finite(`${this}.position.y`, value.y);
    }

    /** Magnification: 1 shows the content at its size. */
    get zoom(): number {
        return this.read<number[]>("scale")[0];
    }
    set zoom(value: number) {
        if (finite(`${this}.zoom`, value) <= 0) {
            throw new RangeError(`${this}.zoom: must be above 0`);
        }
        this.write("scale", [value, value, 1]);
    }

    /** The roll about the view axis, in radians. */
    // @ts-expect-error The specification narrows `Node.rotation` to one angle for a flat camera.
    override get rotation(): number {
        return this.read<number[]>("rotation")[2];
    }
    // @ts-expect-error As the getter above.
    override set rotation(value: number) {
        this.write("rotation", [0, 0, finite(`${this}.rotation`, value)]);
    }
}

/** Whether a camera is the identity view: no pan, zoom 1, no roll. */
export function isIdentity(camera: Camera2D): boolean {
    const { x, y } = camera.position;
    return x === 0 && y === 0 && camera.zoom === 1 && camera.rotation === 0;
}

// ---- Viewport -----------------------------------------------------------------------------------------------------

/** The state of one viewport, read by the manager. */
export interface ViewportState {
    /** The workspace last assigned or last seen shown. */
    content: object | null;
    camera: Camera2D | Camera3D | null;
    clearColor: string;
    display: Display | null;
}

const viewports = new WeakMap<Viewport, ViewportState>();

/** The state of a viewport, for the manager. */
export function viewportState(viewport: Viewport): ViewportState {
    const state = viewports.get(viewport);
    if (state === undefined) {
        throw new TypeError("not a Viewport");
    }
    return state;
}

function isCamera(value: unknown): value is Camera2D | Camera3D {
    return value instanceof Camera2D || value instanceof Camera3D;
}

/**
 * What a display shows and how it looks at it: a workspace (`content`) seen through a camera. Assigning `content` to
 * the viewport of a display shows that workspace on the display, as `gnoblin.workspaces.show` does, and reading it
 * gives what the display shows now (a workspace is on one display at a time). A `Camera3D` is accepted and kept, and
 * the shell reports once per reload that it draws a flat workspace only.
 */
export class Viewport {
    /**
     * @throws TypeError when `content` is not a workspace or `camera` is not a camera
     */
    constructor(options: ViewportInit = {}) {
        const { content, camera, clearColor, ...unknown } = options;
        const extra = Object.keys(unknown);
        if (extra.length > 0) {
            throw new TypeError(`Viewport.${extra[0]}: not a property of a viewport`);
        }
        viewports.set(this, { content: null, camera: null, clearColor: "#000000", display: null });
        if (content !== undefined) this.content = content;
        if (camera !== undefined) this.camera = camera;
        if (clearColor !== undefined) this.clearColor = clearColor;
    }

    /** The workspace the viewport shows. Null for a viewport that was made without one and has no display to ask. */
    get content(): object | null {
        const state = viewportState(this);
        if (state.display === null) {
            return state.content;
        }
        const shown = shell().shown(state.display);
        return shown === undefined ? state.content : shown;
    }
    set content(value: object) {
        if (!shell().isWorkspace(value)) {
            throw new TypeError("Viewport.content: must be a Workspace");
        }
        const state = viewportState(this);
        if (state.display !== null) {
            shell().show(state.display, value);
        }
        state.content = value;
    }

    /** The camera. A viewport made without one has a `Camera2D` with no zoom, pan or roll. */
    get camera(): Camera2D | Camera3D {
        const state = viewportState(this);
        if (state.camera === null) {
            state.camera = new Camera2D();
        }
        return state.camera;
    }
    set camera(value: Camera2D | Camera3D) {
        if (!isCamera(value)) {
            throw new TypeError("Viewport.camera: must be a Camera2D or a Camera3D");
        }
        viewportState(this).camera = value;
    }

    /** The colour behind the content: kept, but nothing draws it yet. */
    get clearColor(): string {
        return viewportState(this).clearColor;
    }
    set clearColor(value: string) {
        if (typeof value !== "string" || value === "") {
            throw new TypeError("Viewport.clearColor: must be a colour such as #101820");
        }
        viewportState(this).clearColor = value;
        shell().changed();
    }

    toString(): string {
        return "Viewport";
    }
}

// ---- Display ------------------------------------------------------------------------------------------------------

/** The state of one display, read and written by the manager. */
export interface DisplayState {
    id: string;
    mode: DisplayModeName;
    selectors: readonly (OutputProfile | OutputView)[];
    viewport: Viewport;
    /** A display the shell made for an output nobody else presents. It is in `gnoblin.displays` until one is. */
    implicit: boolean;
    /** The connected outputs the display presents now, in selector order. */
    live: OutputView[];
    /** The workspace assigned while the display could not show it (parked, or not in the registry yet). */
    wanted: object | null;
    /** The mirrored outputs this display made, and what each mirrors. */
    mirrored: Map<OutputView, OutputView>;
    /** The output this display spans from, while it does. */
    spanLead: OutputView | null;
    /** The `Collection<Output>` of `live`, once someone asked for it. */
    outputs: { collection: DisplayOutputCollection; set(outputs: readonly OutputView[]): void } | null;
}

const displays = new WeakMap<Display, DisplayState>();

/** The state of a display, for the manager. */
export function displayState(display: Display): DisplayState {
    const state = displays.get(display);
    if (state === undefined) {
        throw new TypeError("not a Display");
    }
    return state;
}

const DISPLAY_MODES: readonly string[] = ["single", "mirror", "span"];

function checkMode(mode: unknown, count: number, target: string): DisplayModeName {
    if (typeof mode !== "string" || !DISPLAY_MODES.includes(mode)) {
        throw new TypeError(`${target}.mode: must be "single", "mirror" or "span"`);
    }
    if (mode === "single" && count > 1) {
        throw new TypeError(`${target}.mode: "single" presents one output; use "mirror" or "span" for ${count}`);
    }
    return mode as DisplayModeName;
}

/**
 * A logical presentation: one viewport shown on one output (`single`), copied to several (`mirror`) or stretched over
 * their union (`span`). An output belongs to one display at a time: `gnoblin.displays.add` refuses a display that
 * shares an output with another and names both. With no matching output connected the display is parked and its
 * workspace stays intact.
 *
 * The shell makes a `single` display for every output no script display presents, so `gnoblin.displays` always lists
 * what each monitor shows.
 */
export class Display {
    /**
     * @throws TypeError when `outputs` is empty or holds something else than profiles and outputs, `mode` is unknown or
     *   `viewport` already belongs to another display
     */
    constructor(init: DisplayInit) {
        if (init === null || typeof init !== "object") {
            throw new TypeError("Display: needs an object with outputs and a viewport");
        }
        const { id, outputs, mode, viewport, ...unknown } = init;
        const extra = Object.keys(unknown);
        if (extra.length > 0) {
            throw new TypeError(`Display.${extra[0]}: not a property of a display`);
        }
        if (!Array.isArray(outputs) || outputs.length === 0) {
            throw new TypeError("Display.outputs: must list at least one OutputProfile or Output");
        }
        for (const selector of outputs) {
            if (!(selector instanceof OutputProfile) && !shell().isOutput(selector)) {
                throw new TypeError("Display.outputs: each entry must be an OutputProfile or an Output");
            }
        }
        if (id !== undefined && (typeof id !== "string" || id === "")) {
            throw new TypeError("Display.id: must be a non-empty string");
        }
        if (viewport !== undefined && !(viewport instanceof Viewport)) {
            throw new TypeError("Display.viewport: must be a Viewport");
        }
        const own = viewport ?? new Viewport();
        const owner = viewportState(own).display;
        if (owner !== null) {
            throw new TypeError(`Display.viewport: already shown by display "${owner.id}"`);
        }
        displays.set(this, {
            id: id ?? `display-${++counter}`,
            mode: checkMode(mode ?? "single", outputs.length, "Display"),
            selectors: Object.freeze([...outputs]),
            viewport: own,
            implicit: false,
            live: [],
            wanted: viewportState(own).content,
            mirrored: new Map(),
            spanLead: null,
            outputs: null,
        });
        viewportState(own).display = this;
    }

    get id(): string {
        return displayState(this).id;
    }

    /** `single`, `mirror` or `span`. */
    get mode(): DisplayModeName {
        return displayState(this).mode;
    }
    set mode(value: DisplayModeName) {
        const state = displayState(this);
        state.mode = checkMode(value, state.selectors.length, `Display "${state.id}"`);
        shell().changed();
    }

    /** The connected outputs the display presents now: a live `Collection<Output>`, empty while the display is parked. */
    get outputs(): DisplayOutputCollection {
        return shell().outputs(this);
    }

    get viewport(): Viewport {
        return displayState(this).viewport;
    }

    toString(): string {
        return `Display#${displayState(this).id}`;
    }
}

/** Makes the display the shell keeps for an output no script display presents. Used by the manager only. */
export function implicitDisplay(output: OutputView, id: string): Display {
    const display = new Display({ id, outputs: [output] });
    displayState(display).implicit = true;
    return display;
}
