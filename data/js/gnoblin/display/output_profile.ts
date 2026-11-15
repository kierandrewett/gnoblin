// `OutputProfile`, a persistent selector for one monitor and the settings it gets when connected, with the checked
// state the manager reads through `profileState`.

import { outputRequest } from "gnoblin/display/output_config";
import type { OutputSetting, OutputTransform } from "gnoblin/display/output_config";
import type {
    OutputAlign,
    OutputMatch,
    OutputProfileInit,
    OutputProfileMode,
    OutputProfilePosition,
    OutputView,
    PlacementTarget,
} from "gnoblin/display/types";
import { shell } from "gnoblin/display/state";

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
