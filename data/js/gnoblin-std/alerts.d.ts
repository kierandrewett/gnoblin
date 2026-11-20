import { Box } from "gnoblin-std/ui/box";
import { Text } from "gnoblin-std/ui/text";
import type { UiKeyEvent, UiPointerEvent } from "gnoblin-std/ui/events";
/** What an alert needs of a window. A `Client` of the host module is one. */
export interface AlertClient {
    /** Whether the client still exists. */
    readonly alive: boolean;
    /** Whether the compositor currently marks it unresponsive. */
    readonly unresponsive: boolean;
    /** The client title, which may be empty. */
    readonly title: string;
    /** Whether the client requires a secure alert surface. */
    readonly secure?: boolean;
    /** Requests that the client process be terminated. */
    kill(): void;
}
/** What the user chose, or `recovered` when the alert ended without them. */
export type AlertAnswer = "wait" | "force" | "recovered";
/** Clients to alert and clients whose alerts should close after a tracker check. */
export interface AlertChanges<W> {
    /** The windows that have just stopped answering and get an alert. */
    open: W[];
    /** The windows whose alert is no longer wanted: they answer again, are gone, or lie under a lock. */
    close: W[];
}
/**
 * Which windows get an alert. `check` looks at every window once and says which alerts to open and which to close;
 * `answered` takes the user's choice (or the end of the alert) and says whether to kill the window.
 */
export declare class AlertTracker<W extends AlertClient> {
    #private;
    /** Whether an alert is open for `window`. */
    isOpen(window: W): boolean;
    /** Whether alerts are suppressed until this client answers again. */
    isWaiting(window: W): boolean;
    /** Checks clients and returns alerts to open and close. */
    check(windows: Iterable<W>, locked: boolean): AlertChanges<W>;
    /**
     * The alert for `window` ended with `answer`. Returns whether the window is to be killed. A window the user chose
     * Wait or Force quit for is waited on until it answers a ping again.
     */
    answered(window: W, answer: AlertAnswer): boolean;
}
/**
 * Text from a peer is for display only: no control characters, no lone surrogates, and no more than `max`
 * characters.
 */
export declare function cleanText(text: string, max: number): string;
/** One of the two buttons: Wait or Force quit. A button that is not enabled is dim and does nothing. */
declare class AlertButton extends Box {
    #private;
    readonly label: Text;
    enabled: boolean;
    /** The button the keyboard is on. */
    picked: boolean;
    constructor(dialog: NotRespondingDialog, index: number, label: string);
    refresh(): void;
    arrange(): void;
    onPress(event: UiPointerEvent): void;
}
/**
 * The alert: what is wrong, and Wait and Force quit. `onChosen` is called once with the answer. Force quit is off
 * until `arm()`, which the controller calls `armMs` milliseconds after the alert opened.
 */
export declare class NotRespondingDialog extends Box {
    /** The Wait and Force quit buttons, in keyboard selection order. */
    readonly choices: AlertButton[];
    /** The currently selected button index. */
    index: number;
    /** Whether Force quit is enabled. */
    armed: boolean;
    /** Called when the user chooses Wait or Force quit. */
    onChosen?: (answer: "wait" | "force") => void;
    constructor(name: string, secure: boolean, screen: {
        width: number;
        height: number;
    });
    /** The button the keyboard presses. */
    select(index: number): void;
    /** Lets Force quit be pressed. */
    arm(): void;
    /** Activates a button by its index, unless Force quit is still disarmed. */
    activate(index: number): void;
    /** Keeps pointer presses on the alert from reaching its parent. */
    onPress(event: UiPointerEvent): void;
    /** Handles keyboard selection and activation while the alert has focus. */
    onKey(event: UiKeyEvent): void;
}
/** What the controller needs of the shell around it. */
export interface AlertWorld {
    /** Every window. */
    clients(): Iterable<AlertClient>;
    /** Closes an open window menu: an alert takes the keyboard from it. */
    closeMenu?(): void;
}
/** Options for the alert controller. Times are in milliseconds. */
export interface AlertOptions {
    /** Milliseconds Force quit ignores a press after the alert opens. Default 500. */
    armMs?: number;
    /** Milliseconds between looks at the windows. Default 250: the property is read, not signalled. */
    intervalMs?: number;
}
/**
 * Looks at the windows a few times a second, opens the alert for one that has just stopped answering and closes the
 * alert of one that answers again, has gone, or lies under a lock. `unresponsive` is a property the compositor writes,
 * so the alert reads it: it does not wait for a signal there is none of.
 */
export declare class NotRespondingAlerts {
    #private;
    /** Milliseconds after opening before Force quit becomes available. */
    armMs: number;
    /** Milliseconds between checks for unresponsive clients. */
    intervalMs: number;
    /** Tracks which clients need an alert or are waiting for a response. */
    readonly tracker: AlertTracker<AlertClient>;
    /** Whether the alert service has been started. */
    get started(): boolean;
    /** The alert open for `client`, or `undefined`. */
    cardOf(client: AlertClient): NotRespondingDialog | undefined;
    /** Starts alert checks and frame updates for the shell. */
    start(world: AlertWorld, options?: AlertOptions): void;
    /** One look at the windows, now. */
    check(): void;
}
/** The stock alert, started by `startNotRespondingAlert`. */
export declare const notRespondingAlerts: NotRespondingAlerts;
export {};
