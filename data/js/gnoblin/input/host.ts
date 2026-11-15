// The input module's boundary to the compositor and shell. The Rust host implements these interfaces.

import type { BindingCollection } from "gnoblin/bindings/binding";
import type { BindingClient, Point, Rect } from "gnoblin/bindings/types";
import type { ErrorCode } from "gnoblin/errors";

/**
 * Window-management settings published by a seat. Values reflect the compositor's settings or their defaults.
 */
export interface SeatSettings {
    /** Whether the compositor tiles windows at screen edges. */
    readonly edgeTiling: boolean;
    /** `click`, `hover` or `hover-strict`. */
    readonly focusMode: string;
    /** `allow` or `prevent`. */
    readonly focusNewWindows: string;
    /** Whether a focused window is raised when clicked. */
    readonly raiseOnClick: boolean;
    /** Whether a window raises automatically when the pointer rests on it. */
    readonly autoRaise: boolean;
    /** Delay before automatic raising, in milliseconds. */
    readonly autoRaiseDelay: number;
    /** Whether pointer rest can change keyboard focus. */
    readonly focusChangeOnPointerRest: boolean;
    /** The user asked for reduced motion: the shell's animations take no time. */
    readonly reducedMotion: boolean;
}

/** What a seat says and does, as plain functions over node ids. The shell's Rust host implements it. */
export interface InputHost {
    /** Returns the ids of seats currently reported by the compositor. */
    seats(): number[];
    /** Returns the display name for a seat id. */
    seatName(seat: number): string;
    /** The pointer in the compositor's logical space. */
    pointer(seat: number): [number, number];
    /** The node that has keyboard focus, or 0. */
    focus(seat: number): number;
    /** The seat's devices, one `id:name:caps` item each. */
    devices(seat: number): string[];
    /** The seat's `drag_constraint`: `work_area` or `minimum_strip`. */
    dragConstraint(seat: number): string;
    /** The seat's window management settings, or undefined for a seat the tree no longer has. */
    settings(seat: number): SeatSettings | undefined;
    /** Queues the seat's keyboard focus going to no window; a refusal code, or undefined when queued. */
    clearFocus(seat: number): string | undefined;
    /**
     * Queues the seat's `decorations_available`: whether this shell draws window decorations. The compositor tells a
     * client that asks for server-side that the server draws only while it is true. A refusal code, or undefined
     * when queued.
     */
    declareDecorations(seat: number, available: boolean): string | undefined;
    /** A claim id, or a refusal code. */
    claim(seat: number, chord: string, grab: boolean, device: string): number | string;
    /** Requests a compositor claim for a touchpad gesture. */
    claimGesture(seat: number, kind: string, fingers: number, device: string): number | string;
    /** Makes the devices that report `device` binding-only: a claim id, or a refusal code. */
    claimDevice(seat: number, device: string): number | string;
    /** Releases a claim previously returned by a claim method. */
    unclaim(seat: number, id: number): void;
    /** Milliseconds on a clock that only goes forward. */
    now(): number;
}

/** What the module needs of the clients and of the shell's call queue. */
export interface InputWorld {
    /** Returns the client with the given compositor node id, or undefined if it is not known. */
    clientOf(node: number): BindingClient | undefined;
    /** Every client, back to front. */
    stack(): BindingClient[];
    /**
     * The rectangle a dragged window stays in (the work area of its output), or null when the tree has no screen yet.
     */
    workArea(client?: BindingClient): Rect | null;
    /** The display under `point` (the primary output's when none is), or null when no display has an output. */
    displayAt(point: Point): unknown;
    /** The layouts the workspaces use now. Their bindings go live in every context while they are in use. */
    layouts(): InputLayout[];
    /** Runs `run` later, under the call budget. A throw is reported against `site`. */
    schedule(site: string, run: () => void): void;
    /** Reports an error from a scheduled hook without interrupting other calls. */
    report(site: string, error: unknown): void;
    /** Creates an error for the given code, target and message. */
    error(code: ErrorCode, target: string, message: string): Error;
}

/** What a seat the host has no record of reads as: the schema defaults. */
export const SEAT_DEFAULTS: SeatSettings = Object.freeze({
    edgeTiling: false,
    focusMode: "click",
    focusNewWindows: "prevent",
    raiseOnClick: true,
    autoRaise: false,
    autoRaiseDelay: 500,
    focusChangeOnPointerRest: false,
    reducedMotion: false,
});

/** The layout side of a registration: the input module reads its bindings. */
export interface InputLayout {
    /** Bindings registered for clients that use this layout. */
    readonly bindings: BindingCollection;
}

/** The result of a hit test: the topmost client under a point. */
export interface InputHit {
    /** The client found at the tested position. */
    readonly client: BindingClient;
    /** The hit position relative to the client's top-left corner, in logical pixels. */
    readonly localPosition: Point;
}
