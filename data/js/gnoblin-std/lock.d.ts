import { Box } from "gnoblin-std/ui/box";
import type { LockBackend } from "gnoblin-core/lock";
import type { UiKeyEvent } from "gnoblin-std/ui/events";
import type { PaintOp } from "gnoblin-std/ui/paint";
/** Everything that sets how the screen looks and what it says. */
export interface LockStyle {
    /** Colour used behind the lock card. */
    scrimFill: string;
    /** Card width in logical pixels. */
    cardWidth: number;
    /** Card height in logical pixels. */
    cardHeight: number;
    /** Card corner radius in logical pixels. */
    cardRadius: number;
    /** Space inside the card, in logical pixels. */
    cardPadding: number;
    /** Space between card contents, in logical pixels. */
    cardGap: number;
    /** Card background colour. */
    cardFill: string;
    /** The card's distance from the top, as a share of the screen's height. */
    cardTop: number;
    /** Clock text size in logical pixels. */
    clockSize: number;
    /** Clock text colour. */
    clockColor: string;
    /** Password entry height in logical pixels. */
    entryHeight: number;
    /** Password entry corner radius in logical pixels. */
    entryRadius: number;
    /** Password entry background colour. */
    entryFill: string;
    /** Password entry text size in logical pixels. */
    entrySize: number;
    /** Password entry text colour. */
    entryColor: string;
    /** The most characters the entry takes. The compositor refuses a password over 1024 bytes. */
    entryLimit: number;
    /** Message text size in logical pixels. */
    messageSize: number;
    /** Hint text colour. */
    hintColor: string;
    /** Error text colour. */
    errorColor: string;
    /** Message shown before a password is submitted. */
    hint: string;
    /** Message shown while the compositor checks a password. */
    checking: string;
    /** Message shown when a password is rejected. */
    denied: string;
    /** Message shown when another password check is still running. */
    busy: string;
    /** Message shown when the password check fails. */
    failed: string;
    /** The text of the rate-limit message; `{seconds}` is the wait, rounded up. */
    rateLimited: string;
}
/** The stock look of the lock screen. Used unless the shell passes its own `LockStyle`. Lengths are pixels. */
export declare const defaultLockStyle: Readonly<LockStyle>;
/**
 * The password entry. It holds the typed characters as code points and draws one mask character for each. Enter hands
 * the password, as one string, to `onSubmit` and then overwrites what it held; a key typed while the entry is not
 * `enabled` is dropped.
 */
export declare class PasswordEntry extends Box {
    #private;
    /** Whether the entry takes keys. The lock screen turns it off before it comes down. */
    enabled: boolean;
    /** Maximum typed code points. */
    limit: number;
    /** Mask text size in logical pixels. */
    size: number;
    /** Mask text colour. */
    color: string;
    /** Receives the password on Enter; the entry clears its characters after the callback returns or throws. */
    onSubmit: ((password: string) => void) | null;
    constructor(style: LockStyle, width: number);
    /** How many characters are typed. */
    get length(): number;
    /** Forgets what was typed: the code points are overwritten, then dropped. */
    clear(): void;
    /** Handles typed characters, Enter and Backspace while enabled. */
    onKey(event: UiKeyEvent): void;
    /** Draws the entry and one mask character for each typed code point. */
    draw(): PaintOp[];
}
/** The time of day as `HH:MM`, in the machine's time zone. */
export declare function formatClock(milliseconds: number): string;
/**
 * The lock screen. `frame()` is called once a frame by the host: it takes the compositor's answer to a password check,
 * puts the screen up or takes it down by the lock's state, and ignores every key once the session is unlocked.
 */
export declare class LockScreen {
    #private;
    /** The style values used to draw this screen. */
    readonly style: LockStyle;
    constructor(backend: LockBackend, style?: LockStyle);
    /** Whether the screen is up. */
    get shown(): boolean;
    /** Updates lock state and handles password answers once per host frame. */
    frame(): void;
    /**
     * Renews the compositor's lease on the screen. The host calls it after the toolkit has drawn the frame, and only
     * when `frame()` ran to its end. A turn is healthy, and renews, when the screen is up, every canvas of it is still
     * in the tree, and no refresh, arrange or draw of one of them failed in the turn (a `draw()` that throws, a write
     * the compositor refused). A screen that cannot draw is not one a password can be typed into, so a turn that
     * failed renews nothing and the compositor's own card takes the password when the lease lapses; the first healthy
     * turn after it renews again. A handler that throws on one key is that key's failure and is reported; it is not
     * a turn that failed to draw.
     */
    renew(): void;
    /** Takes the screen down at once, as a generation that is going away does. */
    release(): void;
}
/** Connects the lock screen to the shell host. */
export declare function attachLock(backend: LockBackend): void;
/** The lock screen of this generation, once `attachLock` ran. For tests and diagnostics. */
export declare function lockScreen(): LockScreen | null;
/** Takes the lock screen down. A generation that is replaced calls this, so the next one starts from nothing. */
export declare function lockRelease(): void;
/**
 * One frame of the lock screen and of the toolkit that draws it, then `after` (what the host runs once the toolkit has
 * drawn: the XR scene's frame). A failure of the screen is passed to `onError` and does not stop the toolkit's frame.
 * The lease on the screen is renewed last, after the toolkit has drawn it: a frame that threw, or that drew the screen
 * with a failure, renews nothing (see `LockScreen.renew`).
 */
export declare function runLockFrame(onError: (site: string, error: unknown) => void, after?: () => void): void;
