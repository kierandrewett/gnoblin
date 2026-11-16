// The stock lock screen shows a clock and password entry while the session is locked. It reads the lock node's
// `state` and sends passwords to the compositor with `authenticate`. A JavaScript string cannot be wiped, so the copy
// passed to `authenticate` remains on the engine heap until collected. `PasswordEntry` overwrites its own characters
// on submit and draws one mask character for each. Without a shell lock screen, the Rust fallback card is used
// (`docs/session-lock.md`).

import { Box } from "gnoblin-std/ui/box";
import { runUiFrame } from "gnoblin-std/ui/frame";
import { uiFocus, uiFocused } from "gnoblin-std/ui/session";
import { Text } from "gnoblin-std/ui/text";
import { NEVER, NOW, onWake } from "gnoblin-core/wake";
import type { LockAnswer, LockBackend } from "gnoblin-core/lock";
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
export const defaultLockStyle: Readonly<LockStyle> = {
    scrimFill: "#11111bf2",
    cardWidth: 360,
    cardHeight: 220,
    cardRadius: 16,
    cardPadding: 20,
    cardGap: 12,
    cardFill: "#1e1e2e",
    cardTop: 1 / 3,
    clockSize: 44,
    clockColor: "#cdd6f4",
    entryHeight: 44,
    entryRadius: 10,
    entryFill: "#313244",
    entrySize: 15,
    entryColor: "#cdd6f4",
    entryLimit: 1024,
    messageSize: 13,
    hintColor: "#a6adc8",
    errorColor: "#f38ba8",
    hint: "Enter your password.",
    checking: "Checking...",
    denied: "Wrong password.",
    busy: "Checking the previous attempt.",
    failed: "The password could not be checked.",
    rateLimited: "Too many attempts. Try again in {seconds} s.",
};

/**
 * The password entry. It holds the typed characters as code points and draws one mask character for each. Enter hands
 * the password, as one string, to `onSubmit` and then overwrites what it held; a key typed while the entry is not
 * `enabled` is dropped.
 */
export class PasswordEntry extends Box {
    #typed: number[] = [];
    /** Whether the entry takes keys. The lock screen turns it off before it comes down. */
    enabled = true;
    /** Maximum typed code points. */
    limit: number;
    /** Mask text size in logical pixels. */
    size: number;
    /** Mask text colour. */
    color: string;
    /** Receives the password on Enter; the entry clears its characters after the callback returns or throws. */
    onSubmit: ((password: string) => void) | null = null;

    constructor(style: LockStyle, width: number) {
        super({
            width,
            height: style.entryHeight,
            fill: style.entryFill,
            radius: style.entryRadius,
        });
        this.limit = style.entryLimit;
        this.size = style.entrySize;
        this.color = style.entryColor;
    }

    /** How many characters are typed. */
    get length(): number {
        return this.#typed.length;
    }

    /** Forgets what was typed: the code points are overwritten, then dropped. */
    clear(): void {
        this.#typed.fill(0);
        this.#typed.length = 0;
    }

    /** Handles typed characters, Enter and Backspace while enabled. */
    override onKey(event: UiKeyEvent): void {
        if (!this.enabled || !event.pressed) {
            return;
        }
        if (event.key === "Return" || event.key === "KP_Enter") {
            this.#submit();
        } else if (event.key === "BackSpace") {
            this.#typed.pop();
        } else {
            for (const char of event.text) {
                const code = char.codePointAt(0) as number;
                if (code >= 0x20 && code !== 0x7f && this.#typed.length < this.limit) {
                    this.#typed.push(code);
                }
            }
        }
    }

    #submit(): void {
        if (this.#typed.length === 0 || this.onSubmit === null) {
            return;
        }
        try {
            this.onSubmit(String.fromCodePoint(...this.#typed));
        } finally {
            this.clear();
        }
    }

    /** Draws the entry and one mask character for each typed code point. */
    override draw(): PaintOp[] {
        const ops = super.draw();
        const mask = "*".repeat(this.#typed.length);
        const y = Math.floor((this.height - Math.floor((this.size * 3) / 2)) / 2);
        ops.push({ op: "text", text: mask, x: 12, y, size: this.size, color: this.color });
        return ops;
    }
}

/** The time of day as `HH:MM`, in the machine's time zone. */
export function formatClock(milliseconds: number): string {
    const when = new Date(milliseconds);
    const two = (value: number): string => String(value).padStart(2, "0");
    return `${two(when.getHours())}:${two(when.getMinutes())}`;
}

/** A text that shows the time of day and follows it: `refresh` reads the clock once a frame. */
class ClockText extends Text {
    override refresh(): void {
        this.text = formatClock(Date.now());
    }
}

/** What the screen made while it is up. */
interface Shown {
    scrim: Box;
    clock: ClockText;
    entry: PasswordEntry;
    message: Text;
    /** The node that had the keyboard before the screen took it, given back when the screen comes down. */
    before: number | null;
}

/**
 * The lock screen. `frame()` is called once a frame by the host: it takes the compositor's answer to a password check,
 * puts the screen up or takes it down by the lock's state, and ignores every key once the session is unlocked.
 */
export class LockScreen {
    readonly #backend: LockBackend;
    /** The style values used to draw this screen. */
    readonly style: LockStyle;
    #shown: Shown | null = null;
    /** The token of the check in flight. A second Enter while it runs is not a second attempt. */
    #pending: number | null = null;
    /** A password was accepted: nothing is drawn or heard until the state itself says `not-locked`. */
    #accepted = false;
    /** The generation is going away: the screen is down for good and never comes back. */
    #released = false;
    /** How many steps of the update pass of the screen's canvases had failed when the lease was last considered. */
    #failuresSeen = 0;

    constructor(backend: LockBackend, style: LockStyle = defaultLockStyle) {
        this.#backend = backend;
        this.style = style;
        // The screen renews the compositor's lease and follows the clock and the password entry every frame it is up.
        onWake("lock screen", () => (this.#shown === null || this.#released ? NEVER : NOW));
    }

    /** Whether the screen is up. */
    get shown(): boolean {
        return this.#shown !== null;
    }

    /** Updates lock state and handles password answers once per host frame. */
    frame(): void {
        if (this.#released) {
            return;
        }
        for (const answer of this.#backend.takeAnswers()) {
            if (answer.token === this.#pending) {
                this.#settle(answer);
            }
        }
        if (this.#backend.state() === "not-locked") {
            this.#accepted = false;
            this.#hide();
        } else if (this.#shown === null && !this.#accepted) {
            this.#show();
        } else if (this.#shown !== null) {
            this.#holdKeyboard(this.#shown);
        }
    }

    /**
     * Renews the compositor's lease on the screen. The host calls it after the toolkit has drawn the frame, and only
     * when `frame()` ran to its end. A turn is healthy, and renews, when the screen is up, every canvas of it is still
     * in the tree, and no refresh, arrange or draw of one of them failed in the turn (a `draw()` that throws, a write
     * the compositor refused). A screen that cannot draw is not one a password can be typed into, so a turn that
     * failed renews nothing and the compositor's own card takes the password when the lease lapses; the first healthy
     * turn after it renews again. A handler that throws on one key is that key's failure and is reported; it is not
     * a turn that failed to draw.
     */
    renew(): void {
        const shown = this.#shown;
        if (shown === null || this.#released) {
            return;
        }
        const failures = shown.scrim.failures;
        const drew = shown.scrim.alive && failures <= this.#failuresSeen;
        this.#failuresSeen = failures;
        if (drew) {
            this.#backend.heartbeat();
        }
    }

    /** Takes the screen down at once, as a generation that is going away does. */
    release(): void {
        this.#released = true;
        this.#hide();
    }

    /**
     * The screen is modal: while it is up the keyboard is on its entry. Something else may take it (a window that
     * arrived under the lock and was given the focus by the stock policy, or a script), and the keys then go nowhere
     * the screen can read. The screen takes the keyboard back, and a window that took it is the one that has it when
     * the screen comes down, as it would have had it without the lock.
     */
    #holdKeyboard(shown: Shown): void {
        const held = uiFocused();
        if (held === shown.entry.node) {
            return;
        }
        if (held !== null) {
            shown.before = held;
        }
        try {
            uiFocus(shown.entry.node);
        } catch {
            // The compositor refused the focus (the tree has no seat): the compositor's card takes the keys.
        }
    }

    #show(): void {
        const size = this.#backend.screen();
        const lock = this.#backend.node();
        if (size === undefined || lock === undefined) {
            return; // the tree has no screen yet: the lock screen waits for it
        }
        const [width, height] = size;
        const style = this.style;
        const scrim = new Box({ width, height, fill: style.scrimFill });
        scrim.mount(lock);
        const card = new Box({
            x: Math.floor((width - style.cardWidth) / 2),
            y: Math.floor(height * style.cardTop),
            width: style.cardWidth,
            height: style.cardHeight,
            radius: style.cardRadius,
            fill: style.cardFill,
            padding: style.cardPadding,
            gap: style.cardGap,
            direction: "column",
        });
        scrim.add(card);
        const clock = new ClockText({ size: style.clockSize, color: style.clockColor });
        clock.refresh();
        card.add(clock);
        const entry = new PasswordEntry(style, style.cardWidth - 2 * style.cardPadding);
        card.add(entry);
        const message = new Text({ text: style.hint, size: style.messageSize, color: style.hintColor });
        card.add(message);
        const before = uiFocused();
        entry.onSubmit = (password) => this.#submit(password);
        this.#shown = { scrim, clock, entry, message, before };
        this.#failuresSeen = scrim.failures;
        try {
            uiFocus(entry.node);
        } catch {
            // The compositor refused the focus (the tree has no seat yet): the screen is up all the same.
        }
        this.#backend.engage(true);
    }

    #submit(password: string): void {
        const shown = this.#shown;
        if (shown === null || this.#pending !== null || this.#accepted) {
            return; // an Enter pressed while a check runs is not a second attempt
        }
        const token = this.#backend.authenticate(password);
        if (token === undefined) {
            this.#say(this.style.failed, this.style.errorColor);
            return;
        }
        this.#pending = token;
        this.#say(this.style.checking, this.style.hintColor);
    }

    #settle(answer: LockAnswer): void {
        this.#pending = null;
        if (answer.result === "ok" && answer.error === "") {
            // The compositor has unlocked the session. The screen comes down now; the state follows.
            this.#accepted = true;
            this.#hide();
            return;
        }
        this.#say(this.#feedback(answer), this.style.errorColor);
    }

    #feedback(answer: LockAnswer): string {
        const style = this.style;
        switch (answer.error) {
            case "rate-limited":
                return style.rateLimited.replace("{seconds}", String(Math.ceil((Number(answer.result) || 0) / 1000)));
            case "denied":
                return style.denied;
            case "busy":
                return style.busy;
            default:
                return style.failed;
        }
    }

    #say(text: string, color: string): void {
        if (this.#shown !== null) {
            this.#shown.message.text = text;
            this.#shown.message.color = color;
        }
    }

    #hide(): void {
        const shown = this.#shown;
        if (shown === null) {
            return;
        }
        this.#shown = null;
        this.#pending = null;
        shown.entry.enabled = false;
        shown.entry.clear();
        shown.entry.onSubmit = null;
        // Focus goes back to the window that had it only if the screen still held it: a menu or another window that
        // took the focus while the screen was going down keeps it.
        const held = uiFocused() === shown.entry.node;
        shown.scrim.destroy();
        if (held && shown.before !== null) {
            try {
                uiFocus(shown.before);
            } catch {
                // The window that had the keyboard is gone: nothing to give it back to.
            }
        }
        this.#backend.engage(false);
    }
}

let screen: LockScreen | null = null;

/** Connects the lock screen to the shell host. */
export function attachLock(backend: LockBackend): void {
    screen = new LockScreen(backend);
}

/** The lock screen of this generation, once `attachLock` ran. For tests and diagnostics. */
export function lockScreen(): LockScreen | null {
    return screen;
}

/** Takes the lock screen down. A generation that is replaced calls this, so the next one starts from nothing. */
export function lockRelease(): void {
    screen?.release();
}

/**
 * One frame of the lock screen and of the toolkit that draws it, then `after` (what the host runs once the toolkit has
 * drawn: the XR scene's frame). A failure of the screen is passed to `onError` and does not stop the toolkit's frame.
 * The lease on the screen is renewed last, after the toolkit has drawn it: a frame that threw, or that drew the screen
 * with a failure, renews nothing (see `LockScreen.renew`).
 */
export function runLockFrame(onError: (site: string, error: unknown) => void, after: () => void = () => {}): void {
    let ran = false;
    try {
        screen?.frame();
        ran = true;
    } catch (error) {
        onError("LockScreen.frame", error);
    }
    runUiFrame();
    if (ran) {
        try {
            screen?.renew();
        } catch (error) {
            onError("LockScreen.renew", error);
        }
    }
    after();
}
