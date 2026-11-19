// `OutputProfile`, a persistent selector for one monitor and the settings it gets when connected. The library applies it
// (`libgnoblin-std`, `displays.rs`): this class checks what a script wrote, keeps it in the form it was written, and tells the
// library's profile what it says.

import { outputRequest } from "gnoblin-core/output_config";
import type { OutputSetting, OutputTransform } from "gnoblin-core/output_config";
import type {
    OutputAlign,
    OutputMatch,
    OutputProfileInit,
    OutputProfileMode,
    OutputProfilePosition,
    PlacementTarget,
} from "gnoblin-std/display/types";
import { shell } from "gnoblin-std/display/state";
import { Profile as NativeProfile } from "gnoblin-std/generated/classes";
import type { ProfileSpec } from "gnoblin-std/generated/records";
import { AlignKind, PositionKind, ProfileFlags, SelectKind } from "gnoblin-std/generated/constants";

// ---- OutputProfile ------------------------------------------------------------------------------------------------

const MATCH_KEYS: readonly (keyof OutputMatch)[] = ["connector", "make", "model", "serial"];
const SIDES = ["rightOf", "leftOf", "above", "below"] as const;
const ALIGN_ALONG: Record<(typeof SIDES)[number], readonly OutputAlign[]> = {
    rightOf: ["top", "center", "bottom"],
    leftOf: ["top", "center", "bottom"],
    above: ["left", "center", "right"],
    below: ["left", "center", "right"],
};

/**
 * The state of one profile as the script gave it, and the library's profile, which applies it. The library holds what is applied
 * (`libgnoblin-std`, `displays.rs`); this is what the properties read back, in the form the script wrote them.
 */
export interface ProfileState {
    /** Stable profile identifier used in collection paths. */
    id: string;
    /** Hardware fields that must match a connected output. */
    match: Readonly<OutputMatch>;
    /** Requested output mode, if set. */
    mode: OutputProfileMode | undefined;
    /** Requested scale, from 0.5 to 4, if set. */
    scale: number | undefined;
    /** Composition supersampling factor, from 1 to 16, if set. */
    supersample: number | undefined;
    /** Requested output rotation and flip, if set. */
    transform: OutputTransform | undefined;
    /** Whether variable refresh rate is requested. */
    vrr: boolean | undefined;
    /** Requested pixel position or relative placement, if set. */
    position: OutputProfilePosition | undefined;
    /** Whether the output is enabled, if set. */
    enabled: boolean | undefined;
    /** The library's profile. */
    native: NativeProfile;
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
 * A persistent set of settings for a monitor, selected by its hardware details. It is inactive while no matching
 * output is connected and applies when one connects, including after a hotplug. The compositor applies settings as a
 * transaction: if a mode or layout is refused, the last good arrangement remains and the refusal is reported.
 *
 * Properties other than `match` can be changed later; changes apply on the next frame. Removing a profile leaves the
 * last arrangement in place.
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
        // The settings are checked one by one, so the message names the key that is wrong, and the library's profile is made
        // from what they said.
        const state: Omit<ProfileState, "native"> = {
            id: name,
            match: checked,
            mode: undefined,
            scale: undefined,
            supersample: undefined,
            transform: undefined,
            vrr: undefined,
            position: undefined,
            enabled: undefined,
        };
        profiles.set(this, state as ProfileState);
        if (mode !== undefined) this.#check("mode", mode, { mode: modeText(mode, `OutputProfile "${name}"`) });
        if (scale !== undefined) this.#check("scale", scale, { scale });
        if (supersample !== undefined) this.#check("supersample", supersample, { supersample });
        if (transform !== undefined) this.#check("transform", transform, { transform });
        if (vrr !== undefined) this.#checkVrr(vrr);
        if (position !== undefined) this.#checkPosition(position);
        if (enabled !== undefined) this.#check("enabled", enabled, { enabled });
        const native = NativeProfile.new({ ...specOf(state as ProfileState), id: name, ...matchOf(checked) });
        if (native === null) {
            throw new TypeError("OutputProfile: the library made no profile");
        }
        (state as ProfileState).native = native;
    }

    /** The profile's name in `gnoblin.outputs.profiles`. */
    get id(): string {
        return profileState(this).id;
    }

    /** Which monitor the profile is for. Fixed at construction. */
    get match(): Readonly<OutputMatch> {
        return profileState(this).match;
    }

    /** The requested display mode, if set. */
    get mode(): OutputProfileMode | undefined {
        return profileState(this).mode;
    }
    /** Sets the display mode requested by this profile. */
    set mode(value: OutputProfileMode) {
        this.#change("mode", value, { mode: modeText(value, `OutputProfile "${this.id}"`) });
    }

    /** The requested output scale, from 0.5 to 4, if set. */
    get scale(): number | undefined {
        return profileState(this).scale;
    }
    /** Sets the output scale from 0.5 to 4. */
    set scale(value: number) {
        this.#change("scale", value, { scale: value });
    }

    /** The composition supersampling factor, from 1 to 16, if set. */
    get supersample(): number | undefined {
        return profileState(this).supersample;
    }
    /** Sets the composition supersampling factor from 1 to 16. */
    set supersample(value: number) {
        this.#change("supersample", value, { supersample: value });
    }

    /** The requested output transform, if set. */
    get transform(): OutputTransform | undefined {
        return profileState(this).transform;
    }
    /** Sets the output transform. */
    set transform(value: OutputTransform) {
        this.#change("transform", value, { transform: value });
    }

    /** Whether variable refresh rate is asked for. Reported as not applied. */
    get vrr(): boolean | undefined {
        return profileState(this).vrr;
    }
    /** Requests whether variable refresh rate is enabled. */
    set vrr(value: boolean) {
        this.#checkVrr(value);
        this.#push();
    }

    /** The output position, in logical pixels, or a relative placement. */
    get position(): OutputProfilePosition | undefined {
        return profileState(this).position;
    }
    /** Sets an absolute or relative position in logical pixels. */
    set position(value: OutputProfilePosition) {
        this.#checkPosition(value);
        this.#push();
    }

    /** Whether the output is enabled, if set. */
    get enabled(): boolean | undefined {
        return profileState(this).enabled;
    }
    /** Sets whether the output is enabled. */
    set enabled(value: boolean) {
        this.#change("enabled", value, { enabled: value });
    }

    #change<K extends "mode" | "scale" | "supersample" | "transform" | "enabled">(
        key: K,
        value: ProfileState[K],
        check: OutputSetting,
    ): void {
        this.#check(key, value, check);
        this.#push();
    }

    /** Checks a setting as `Output.apply` does (a bad value throws, naming `OutputProfile "id".key`) and keeps it. */
    #check<K extends "mode" | "scale" | "supersample" | "transform" | "enabled">(
        key: K,
        value: ProfileState[K],
        check: OutputSetting,
    ): void {
        outputRequest(check, `OutputProfile "${this.id}"`);
        profileState(this)[key] = value;
    }

    #checkVrr(value: boolean): void {
        if (typeof value !== "boolean") {
            throw new TypeError(`OutputProfile "${this.id}".vrr: must be true or false`);
        }
        profileState(this).vrr = value;
    }

    #checkPosition(value: OutputProfilePosition): void {
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
    }

    /** Tells the library's profile what the profile says now. */
    #push(): void {
        const state = profileState(this);
        state.native.set(specOf(state));
    }

    /** Formats the profile as `OutputProfile#id`. */
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

/** The library's profile spec of a profile's settings: the flags and values the library reads, and nothing it applies on its own. */
function specOf(state: ProfileState): Partial<ProfileSpec> & { id?: string } {
    let flags = 0;
    if (state.scale !== undefined) flags |= ProfileFlags.HAS_SCALE;
    if (state.supersample !== undefined) flags |= ProfileFlags.HAS_SUPERSAMPLE;
    if (state.vrr !== undefined) flags |= ProfileFlags.HAS_VRR | (state.vrr ? ProfileFlags.VRR_ON : 0);
    if (state.enabled !== undefined) flags |= ProfileFlags.HAS_ENABLED | (state.enabled ? ProfileFlags.ENABLED_ON : 0);
    const spec: Partial<ProfileSpec> = {
        flags,
        mode: state.mode === undefined ? "" : modeText(state.mode, `OutputProfile "${state.id}"`),
        transform: state.transform ?? "",
        scale: state.scale ?? 0,
        supersample: state.supersample ?? 0,
        position: PositionKind.NONE,
    };
    const position = state.position;
    if (position !== undefined) {
        const relative = relativeTarget(position);
        if (relative === null) {
            const pixel = position as { x: number; y: number };
            Object.assign(spec, { position: PositionKind.ABSOLUTE, x: pixel.x, y: pixel.y });
        } else {
            const target = relative.target;
            const profile = target instanceof OutputProfile;
            Object.assign(spec, {
                position: PositionKind[SIDE_KIND[relative.side]],
                align: alignOf(relative.align),
                targetKind: profile ? SelectKind.PROFILE : SelectKind.OUTPUT,
                target: profile ? profileState(target).native.handle : shell().outputNode(target),
            });
        }
    }
    return spec;
}

const SIDE_KIND = { rightOf: "RIGHT_OF", leftOf: "LEFT_OF", above: "ABOVE", below: "BELOW" } as const;

/** The side of the shared edge an alignment names: the start (top or left), the middle or the end (bottom or right). */
function alignOf(align: OutputAlign | undefined): number {
    if (align === "center") return AlignKind.CENTER;
    if (align === "bottom" || align === "right") return AlignKind.END;
    return AlignKind.START;
}

/** The match keys of a profile, as the library's spec names them (empty for a key the profile does not match on). */
function matchOf(match: Readonly<OutputMatch>): Pick<ProfileSpec, "connector" | "make" | "model" | "serial"> {
    return {
        connector: match.connector ?? "",
        make: match.make ?? "",
        model: match.model ?? "",
        serial: match.serial ?? "",
    };
}
