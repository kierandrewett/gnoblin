// The option and view types of the display classes: what a script passes to `new OutputProfile`, `new Display`,
// `new Viewport` and `new Camera2D`, and what they read of a connected output.

import type { OutputTransform } from "gnoblin/display/output_config";
import type { OutputProfile } from "gnoblin/display/output_profile";
import type { Camera2D } from "gnoblin/display/camera2d";
import type { Viewport } from "gnoblin/display/viewport";
import type { Camera3D } from "gnoblin/scene/camera";

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
