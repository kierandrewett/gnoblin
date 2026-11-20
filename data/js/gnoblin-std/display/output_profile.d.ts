import type { OutputTransform } from "gnoblin-core/output_config";
import type { OutputAlign, OutputMatch, OutputProfileInit, OutputProfileMode, OutputProfilePosition, PlacementTarget } from "gnoblin-std/display/types";
import { Profile as NativeProfile } from "gnoblin-std/generated/classes";
declare const SIDES: readonly ["rightOf", "leftOf", "above", "below"];
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
/** The state of a profile, for the manager. */
export declare function profileState(profile: OutputProfile): ProfileState;
/**
 * A persistent set of settings for a monitor, selected by its hardware details. It is inactive while no matching
 * output is connected and applies when one connects, including after a hotplug. The compositor applies settings as a
 * transaction: if a mode or layout is refused, the last good arrangement remains and the refusal is reported.
 *
 * Properties other than `match` can be changed later; changes apply on the next frame. Removing a profile leaves the
 * last arrangement in place.
 */
export declare class OutputProfile {
    #private;
    /**
     * @throws TypeError when a key is unknown or a value is out of range (the message names the key)
     */
    constructor(init: OutputProfileInit);
    /** The profile's name in `gnoblin.outputs.profiles`. */
    get id(): string;
    /** Which monitor the profile is for. Fixed at construction. */
    get match(): Readonly<OutputMatch>;
    /** The requested display mode, if set. */
    get mode(): OutputProfileMode | undefined;
    /** Sets the display mode requested by this profile. */
    set mode(value: OutputProfileMode);
    /** The requested output scale, from 0.5 to 4, if set. */
    get scale(): number | undefined;
    /** Sets the output scale from 0.5 to 4. */
    set scale(value: number);
    /** The composition supersampling factor, from 1 to 16, if set. */
    get supersample(): number | undefined;
    /** Sets the composition supersampling factor from 1 to 16. */
    set supersample(value: number);
    /** The requested output transform, if set. */
    get transform(): OutputTransform | undefined;
    /** Sets the output transform. */
    set transform(value: OutputTransform);
    /** Whether variable refresh rate is asked for. Reported as not applied. */
    get vrr(): boolean | undefined;
    /** Requests whether variable refresh rate is enabled. */
    set vrr(value: boolean);
    /** The output position, in logical pixels, or a relative placement. */
    get position(): OutputProfilePosition | undefined;
    /** Sets an absolute or relative position in logical pixels. */
    set position(value: OutputProfilePosition);
    /** Whether the output is enabled, if set. */
    get enabled(): boolean | undefined;
    /** Sets whether the output is enabled. */
    set enabled(value: boolean);
    /** Formats the profile as `OutputProfile#id`. */
    toString(): string;
}
/** The side, target and alignment of a relative position, or null for a pixel position or none. */
export declare function relativeTarget(position: OutputProfilePosition | undefined): {
    side: (typeof SIDES)[number];
    target: PlacementTarget;
    align: OutputAlign | undefined;
} | null;
export {};
