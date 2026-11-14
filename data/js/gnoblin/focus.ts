// The focus policy: click, sloppy and follow-mouse focus, whether a new window takes focus, raise on click and
// auto raise. Ported from data/lua/gnoblin/shell/focus.lua (the decisions) and the `Window:on_*` handlers of
// data/lua/gnoblin/shell/init.lua (the wiring); crates/gnoblin-js/tests/stock_focus.rs runs the cases.
//
// The decisions are pure functions of the settings and one event (`focusDecision`), so they are tested with no
// shell. `FocusPolicy` is the wiring: a listener for a collection of clients that turns the pointer edges the
// compositor reports on windows into those events, runs what the decision says, and keeps the two timers (a pointer
// that rests, a delayed raise) on the frame clock.
//
// Settings. The compositor publishes the `window_management` focus keys on the seat (`focus_mode`,
// `focus_new_windows`, `raise_on_click`, `auto_raise`, `auto_raise_delay`, `focus_change_on_pointer_rest`) and they
// are read-only there. `defineConfig({ focus })` overrides a key for the shell; a key the config leaves out is the
// seat's. The seat's words for the modes translate: click is `click`, hover is `sloppy` (focus follows the pointer
// and stays where it was when the pointer goes to the desktop) and hover-strict is `follow-mouse` (focus acts on
// entry only, and goes when the pointer leaves the focused window).

import type { SeatSettings } from "gnoblin/input";
import { rejectUnknown } from "gnoblin/workspace";

/** The focus modes: `click` focuses on a click, `sloppy` follows the pointer, `follow-mouse` acts on entry only. */
export type FocusMode = "click" | "sloppy" | "follow-mouse";
/** Whether a window started from a launcher takes focus (`allow`) or not (`prevent`). */
export type NewWindowPolicy = "allow" | "prevent";

/** The `focus` key of a config. Every key is optional: a key left out is the seat's. */
export interface FocusConfig {
    mode?: FocusMode;
    /** Whether a window started from a launcher takes focus. */
    newWindows?: NewWindowPolicy;
    raiseOnClick?: boolean;
    autoRaise?: boolean;
    /** Milliseconds a pointer stays over a window before auto raise lifts it. */
    autoRaiseDelay?: number;
    /** Whether a pointer that rests on a window focuses it, in the sloppy and follow-mouse modes. */
    changeOnPointerRest?: boolean;
}

/** The focus settings in force. Read-only: build them with `focusSettings` or `seatFocusSettings`. */
export interface FocusSettings {
    readonly mode: FocusMode;
    readonly newWindows: NewWindowPolicy;
    readonly raiseOnClick: boolean;
    readonly autoRaise: boolean;
    readonly autoRaiseDelay: number;
    readonly changeOnPointerRest: boolean;
}

/** The documented defaults (docs/config/configure/window_management.md). */
export const FOCUS_DEFAULTS: FocusSettings = Object.freeze({
    mode: "click",
    newWindows: "prevent",
    raiseOnClick: true,
    autoRaise: false,
    autoRaiseDelay: 500,
    changeOnPointerRest: false,
});

const FOCUS_KEYS = ["mode", "newWindows", "raiseOnClick", "autoRaise", "autoRaiseDelay", "changeOnPointerRest"];
const FOCUS_MODES: readonly string[] = ["click", "sloppy", "follow-mouse"];
const NEW_WINDOW_POLICIES: readonly string[] = ["allow", "prevent"];

function flag(value: unknown, what: string): boolean {
    if (typeof value !== "boolean") {
        throw new TypeError(`${what} must be a boolean, got ${String(value)}`);
    }
    return value;
}

/** A check for one key: the value if it is fine, else a TypeError that names the key. */
const CHECKS: Readonly<Record<string, (value: unknown, where: string) => unknown>> = {
    mode(value, where) {
        if (typeof value !== "string" || !FOCUS_MODES.includes(value)) {
            throw new TypeError(`${where}.mode must be click, sloppy or follow-mouse, got ${String(value)}`);
        }
        return value;
    },
    newWindows(value, where) {
        if (typeof value !== "string" || !NEW_WINDOW_POLICIES.includes(value)) {
            throw new TypeError(`${where}.newWindows must be allow or prevent, got ${String(value)}`);
        }
        return value;
    },
    raiseOnClick: (value, where) => flag(value, `${where}.raiseOnClick`),
    autoRaise: (value, where) => flag(value, `${where}.autoRaise`),
    changeOnPointerRest: (value, where) => flag(value, `${where}.changeOnPointerRest`),
    autoRaiseDelay(value, where) {
        if (typeof value !== "number" || !Number.isFinite(value) || value < 0) {
            throw new TypeError(
                `${where}.autoRaiseDelay must be a non-negative number of milliseconds, got ${String(value)}`,
            );
        }
        return value;
    },
};

/**
 * The full settings: `base` (the documented defaults) with `user` on top. An unknown key or a value of the wrong
 * kind throws, naming `where`, because a typo must not silently change how focus behaves. A key set to `undefined`
 * is a key left out.
 */
export function focusSettings(
    user: FocusConfig = {},
    where = "focus",
    base: FocusSettings = FOCUS_DEFAULTS,
): FocusSettings {
    if (user === null || typeof user !== "object") {
        throw new TypeError(`${where} must be an object`);
    }
    rejectUnknown(user, FOCUS_KEYS, where);
    const out: Record<string, unknown> = { ...base };
    for (const [key, value] of Object.entries(user)) {
        if (value !== undefined) {
            out[key] = CHECKS[key](value, where);
        }
    }
    return Object.freeze(out) as unknown as FocusSettings;
}

const SEAT_MODES: Readonly<Record<string, FocusMode>> = {
    click: "click",
    hover: "sloppy",
    "hover-strict": "follow-mouse",
};

/**
 * The settings the seat publishes, in the library's words. A seat mode the library does not know throws a TypeError.
 */
export function seatFocusSettings(seat: SeatSettings): FocusSettings {
    const mode = Object.hasOwn(SEAT_MODES, seat.focusMode) ? SEAT_MODES[seat.focusMode] : undefined;
    if (mode === undefined) {
        throw new TypeError(`the seat's focus_mode must be click, hover or hover-strict, got ${seat.focusMode}`);
    }
    return focusSettings(
        {
            mode,
            newWindows: seat.focusNewWindows as NewWindowPolicy,
            raiseOnClick: seat.raiseOnClick,
            autoRaise: seat.autoRaise,
            autoRaiseDelay: seat.autoRaiseDelay,
            changeOnPointerRest: seat.focusChangeOnPointerRest,
        },
        "the seat's focus settings",
    );
}

/** Whether a new window takes focus. A window started from a launcher takes it only under `allow`. */
export function opensFocused(settings: FocusSettings, fromLauncher: boolean): boolean {
    return !fromLauncher || settings.newWindows === "allow";
}

/**
 * Whether the window under the pointer takes focus under the focus mode. `hovering` is false over the desktop.
 * `entered` is true when the pointer crossed into the window from outside it; follow-mouse acts only on entry.
 */
export function hoverFocus(settings: FocusSettings, hovering: boolean, entered: boolean): boolean {
    if (settings.mode === "click" || !hovering) {
        return false;
    }
    if (settings.mode === "follow-mouse") {
        return entered;
    }
    return true;
}

/** The auto raise delay in seconds, for the frame clock. */
export function autoRaiseSeconds(settings: FocusSettings): number {
    return settings.autoRaiseDelay / 1000;
}

/** How long the pointer must rest over a window before `changeOnPointerRest` focuses it, in seconds. */
export const REST_SECONDS = 0.2;

/** Whether a pointer rest focuses the window: a hover mode with `changeOnPointerRest` on. */
export function restFocus(settings: FocusSettings): boolean {
    return settings.changeOnPointerRest && settings.mode !== "click";
}

/** Whether the pointer leaving the focused window clears focus (follow-mouse only). */
export function leaveClears(settings: FocusSettings): boolean {
    return settings.mode === "follow-mouse";
}

/**
 * One thing that happened to a window. `held` is whether some window holds a pressed pointer button, which stops
 * hover focus until the release; `heldHere` is whether this window holds it; `focused` is whether this window has
 * the keyboard focus.
 */
export type FocusEvent =
    | { readonly kind: "added"; readonly fromLauncher?: boolean; readonly skipFocus?: boolean }
    | { readonly kind: "enter" | "motion"; readonly held: boolean }
    | { readonly kind: "leave"; readonly focused: boolean; readonly heldHere: boolean }
    | { readonly kind: "press" };

/** What to do about it. A `...After` is a delay in seconds that a leave or a new rest cancels. */
export interface FocusDecision {
    readonly focus: boolean;
    readonly clear: boolean;
    readonly raise: boolean;
    /** Focus the window after this long, unless the pointer left or a button is held by then. */
    readonly focusAfter: number | null;
    /** Raise the window after this long, if it has the focus then. */
    readonly raiseAfter: number | null;
}

const NOTHING: FocusDecision = Object.freeze({
    focus: false,
    clear: false,
    raise: false,
    focusAfter: null,
    raiseAfter: null,
});

/** The decision for `event` under `settings`. Pure. */
export function focusDecision(settings: FocusSettings, event: FocusEvent): FocusDecision {
    switch (event.kind) {
        case "added":
            return {
                ...NOTHING,
                focus: opensFocused(settings, event.fromLauncher === true) && event.skipFocus !== true,
            };
        case "press":
            return { ...NOTHING, focus: true, raise: settings.raiseOnClick };
        case "enter":
        case "motion": {
            if (event.held) {
                return NOTHING;
            }
            const entered = event.kind === "enter";
            const rests = restFocus(settings);
            return {
                ...NOTHING,
                focus: !rests && hoverFocus(settings, true, entered),
                focusAfter: rests ? REST_SECONDS : null,
                // Auto raise starts on entry; motion inside the window leaves a pending raise alone.
                raiseAfter: entered && settings.autoRaise ? autoRaiseSeconds(settings) : null,
            };
        }
        case "leave":
            return { ...NOTHING, clear: !event.heldHere && event.focused && leaveClears(settings) };
        default:
            throw new TypeError(`unknown focus event ${JSON.stringify(event)}`);
    }
}

/** What the policy asks of the shell. `C` is the shell's client object. */
export interface FocusHost<C extends object> {
    /** The settings now: the seat's, with the config's on top. Read on every event, so a change applies at once. */
    settings(): FocusSettings;
    /** Whether `client` has the keyboard focus now. */
    focused(client: C): boolean;
    focus(client: C): void;
    /** Takes the keyboard focus from every window. */
    clearFocus(): void;
    raise(client: C): void;
    /** Milliseconds on a clock that only goes forward. */
    now(): number;
    /** Whether `client` is a window that appeared while the shell ran, not one an earlier generation already had. */
    isArrival(client: C): boolean;
    /** Whether a window rule asked that `client` not take focus when it appears. */
    skipsFocus(client: C): boolean;
    /** Whether `client` opened on a token the launcher minted (`focus.newWindows` then decides). Absent: no. */
    fromLauncher?(client: C): boolean;
    alive(client: C): boolean;
}

interface Pending {
    focusAt: number | null;
    raiseAt: number | null;
}

/**
 * Attach it to `gnoblin.clients`: `onAdded` focuses a new window, and the pointer edges on a window drive hover
 * focus, click focus and raise, and auto raise. Call `frame()` once a frame for the two timers.
 */
export class FocusPolicy<C extends object> {
    readonly #host: FocusHost<C>;
    readonly #pending = new Map<C, Pending>();
    /** The window whose button is held, so focus and rest wait for the release. */
    #pressed: C | null = null;

    constructor(host: FocusHost<C>) {
        this.#host = host;
    }

    onAdded(client: C): void {
        if (!this.#host.isArrival(client)) {
            return;
        }
        const skipFocus = this.#host.skipsFocus(client);
        const fromLauncher = this.#host.fromLauncher?.(client) ?? false;
        this.#run(client, focusDecision(this.#host.settings(), { kind: "added", skipFocus, fromLauncher }));
    }

    onPointerEnter(client: C): void {
        this.#run(client, focusDecision(this.#host.settings(), { kind: "enter", held: this.#pressed !== null }));
    }

    onPointerMotion(client: C): void {
        this.#run(client, focusDecision(this.#host.settings(), { kind: "motion", held: this.#pressed !== null }));
    }

    onPointerLeave(client: C): void {
        this.#pending.delete(client);
        const focused = this.#host.focused(client);
        this.#run(
            client,
            focusDecision(this.#host.settings(), { kind: "leave", focused, heldHere: this.#pressed === client }),
        );
    }

    onPointerPress(client: C): void {
        this.#pressed = client;
        this.#run(client, focusDecision(this.#host.settings(), { kind: "press" }));
    }

    onPointerRelease(): void {
        this.#pressed = null;
    }

    onClosed(client: C): void {
        this.#pending.delete(client);
        if (this.#pressed === client) {
            this.#pressed = null;
        }
    }

    /** Runs the timers that are due: a rest that focuses, a delayed raise. */
    frame(): void {
        if (this.#pending.size === 0) {
            return;
        }
        const now = this.#host.now();
        for (const [client, pending] of [...this.#pending]) {
            if (!this.#host.alive(client)) {
                this.#pending.delete(client);
                continue;
            }
            if (pending.focusAt !== null && now >= pending.focusAt) {
                pending.focusAt = null;
                if (this.#pressed !== client) {
                    this.#host.focus(client);
                }
            }
            if (pending.raiseAt !== null && now >= pending.raiseAt) {
                pending.raiseAt = null;
                if (this.#host.focused(client)) {
                    this.#host.raise(client);
                }
            }
            if (pending.focusAt === null && pending.raiseAt === null) {
                this.#pending.delete(client);
            }
        }
    }

    #run(client: C, decision: FocusDecision): void {
        if (!this.#host.alive(client)) {
            return;
        }
        if (decision.clear) {
            this.#host.clearFocus();
        }
        if (decision.focus) {
            this.#host.focus(client);
        }
        if (decision.raise) {
            this.#host.raise(client);
        }
        if (decision.focusAfter !== null || decision.raiseAfter !== null) {
            const now = this.#host.now();
            const before = this.#pending.get(client);
            // A rest that starts again moves the focus time; a raise that was waiting keeps its own.
            this.#pending.set(client, {
                focusAt: decision.focusAfter === null ? (before?.focusAt ?? null) : now + decision.focusAfter * 1000,
                raiseAt: decision.raiseAfter === null ? (before?.raiseAt ?? null) : now + decision.raiseAfter * 1000,
            });
        }
    }
}
