import type { SeatSettings } from "gnoblin-core/input/host";
/** The focus modes: `click` focuses on a click, `sloppy` follows the pointer, `follow-mouse` acts on entry only. */
export type FocusMode = "click" | "sloppy" | "follow-mouse";
/** Whether a window started from a launcher takes focus (`allow`) or not (`prevent`). */
export type NewWindowPolicy = "allow" | "prevent";
/** The focus settings, every key optional: a key left out is the documented default. */
export interface FocusConfig {
    /** Focus behaviour on pointer entry and click. */
    mode?: FocusMode;
    /** Whether a window started from a launcher takes focus. */
    newWindows?: NewWindowPolicy;
    /** Raises a window when it receives a pointer click. */
    raiseOnClick?: boolean;
    /** Raises a window after the pointer rests over it. */
    autoRaise?: boolean;
    /** Milliseconds a pointer stays over a window before auto raise lifts it. */
    autoRaiseDelay?: number;
    /** Whether a pointer that rests on a window focuses it, in the sloppy and follow-mouse modes. */
    changeOnPointerRest?: boolean;
}
/** The focus settings in force. Read-only: build them with `focusSettings` or `seatFocusSettings`. */
export interface FocusSettings {
    /** Focus behaviour on pointer entry and click. */
    readonly mode: FocusMode;
    /** Whether a newly launched window takes focus. */
    readonly newWindows: NewWindowPolicy;
    /** Whether a click raises its window. */
    readonly raiseOnClick: boolean;
    /** Whether pointer rest raises its window. */
    readonly autoRaise: boolean;
    /** Milliseconds the pointer must rest before a window is raised. */
    readonly autoRaiseDelay: number;
    /** Whether pointer rest changes focus in hover modes. */
    readonly changeOnPointerRest: boolean;
}
/** The documented defaults (docs/config/configure/window_management.md). */
export declare const FOCUS_DEFAULTS: FocusSettings;
/**
 * The full settings: `base` (the documented defaults) with `user` on top. An unknown key or a value of the wrong
 * kind throws, naming `where`, because a typo must not silently change how focus behaves. A key set to `undefined`
 * is a key left out.
 */
export declare function focusSettings(user?: FocusConfig, where?: string, base?: FocusSettings): FocusSettings;
/**
 * The settings the seat publishes, in the library's words. A seat mode the library does not know throws a TypeError.
 */
export declare function seatFocusSettings(seat: SeatSettings): FocusSettings;
/** Whether a new window takes focus. A window started from a launcher takes it only under `allow`. */
export declare function opensFocused(settings: FocusSettings, fromLauncher: boolean): boolean;
/**
 * Whether the window under the pointer takes focus under the focus mode. `hovering` is false over the desktop.
 * `entered` is true when the pointer crossed into the window from outside it; follow-mouse acts only on entry.
 */
export declare function hoverFocus(settings: FocusSettings, hovering: boolean, entered: boolean): boolean;
/** How long the pointer must rest over a window before `changeOnPointerRest` focuses it, in milliseconds. */
export declare const REST_DELAY = 200;
/** Whether a pointer rest focuses the window: a hover mode with `changeOnPointerRest` on. */
export declare function restFocus(settings: FocusSettings): boolean;
/** Whether the pointer leaving the focused window clears focus (follow-mouse only). */
export declare function leaveClears(settings: FocusSettings): boolean;
/**
 * One thing that happened to a window. `held` is whether some window holds a pressed pointer button, which stops
 * hover focus until the release; `heldHere` is whether this window holds it; `focused` is whether this window has
 * the keyboard focus.
 */
export type FocusEvent = {
    readonly kind: "added";
    readonly fromLauncher?: boolean;
} | {
    readonly kind: "enter" | "motion";
    readonly held: boolean;
} | {
    readonly kind: "leave";
    readonly focused: boolean;
    readonly heldHere: boolean;
} | {
    readonly kind: "press";
};
/** What to do about it. A `...After` is a delay in milliseconds that a leave or a new rest cancels. */
export interface FocusDecision {
    /** Focuses the window immediately. */
    readonly focus: boolean;
    /** Clears focus from all windows. */
    readonly clear: boolean;
    /** Raises the window immediately. */
    readonly raise: boolean;
    /** Focus the window after this long, unless the pointer left or a button is held by then. */
    readonly focusAfter: number | null;
    /** Raise the window after this long, if it has the focus then. */
    readonly raiseAfter: number | null;
}
/** The decision for `event` under `settings`. Pure. */
export declare function focusDecision(settings: FocusSettings, event: FocusEvent): FocusDecision;
/** What the policy asks of the shell. `C` is the shell's client object. */
export interface FocusHost<C extends object> {
    /** The settings now: the seat's. Read on every event, so a change applies at once. */
    settings(): FocusSettings;
    /** Whether `client` has the keyboard focus now. */
    focused(client: C): boolean;
    /** Gives keyboard focus to `client`. */
    focus(client: C): void;
    /** Takes the keyboard focus from every window. */
    clearFocus(): void;
    /** Raises `client` above other windows. */
    raise(client: C): void;
    /** Milliseconds on a clock that only goes forward. */
    now(): number;
    /** Whether `client` is a window that appeared while the shell ran, not one an earlier generation already had. */
    isArrival(client: C): boolean;
    /** Whether `client` opened on a token the launcher minted (`focus.newWindows` then decides). Absent: no. */
    fromLauncher?(client: C): boolean;
    /** Whether `client` still exists. */
    alive(client: C): boolean;
}
/**
 * Attach it to `gnoblin.clients`: `onAdded` focuses a new window, and the pointer edges on a window drive hover
 * focus, click focus and raise, and auto raise. Call `frame()` once a frame for the two timers.
 */
export declare class FocusPolicy<C extends object> {
    #private;
    constructor(host: FocusHost<C>);
    /** Applies the configured policy when a new window appears. */
    onAdded(client: C): void;
    /** Applies the configured policy when the pointer enters `client`. */
    onPointerEnter(client: C): void;
    /** Applies the configured policy when the pointer moves inside `client`. */
    onPointerMotion(client: C): void;
    /** Cancels pending actions and applies the leave policy. */
    onPointerLeave(client: C): void;
    /** Focuses `client` and raises it when the settings require it. */
    onPointerPress(client: C): void;
    /** Clears the pressed-window state after a pointer release. */
    onPointerRelease(): void;
    /** Cancels pending focus and raise actions for a closed window. */
    onClosed(client: C): void;
    /** Runs the timers that are due: a rest that focuses, a delayed raise. */
    frame(): void;
}
