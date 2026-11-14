// The stock lock screen: a clock and a password entry drawn over the cover while the session is locked.
//
// The compositor owns the lock. It covers every output, takes every key and pointer event away from the clients, and
// checks the password itself (PAM, on a worker thread); only an accepted check unlocks. This screen is what the user
// types into: it never decides anything. It reads the `lock` node's `state`, records that it is up in `engaged` (which
// is what lets the compositor route the typed keys to it, and stop drawing its own built-in card), and sends the
// password with the node's `authenticate` call, whose answer is `ok`, `denied`, `rate-limited` (with the wait in
// milliseconds), `busy` or `failed: ...`.
//
// The screen is a tree of canvases mounted under the lock node, so the compositor draws it over the cover. It is
// made when the state leaves `not-locked` (a shell that attaches to a session that is already locked finds the state
// there, so a restart or a late shell shows it too) and destroyed when the state returns to `not-locked`.
//
// The password: the entry keeps what was typed as an array of code points, not as a string, and overwrites it as soon
// as the call is made. The one string it is ever turned into lives only for the call to `authenticate`. A JavaScript
// string cannot be wiped, so that copy, and the one in the key event that carried each character, stay in the engine's
// heap until the collector frees them; that is the limit of this screen, and the Rust built-in card (no script) does
// not have it. The screen never draws, logs or stores the password: the entry draws one `*` per character.

import { Box, Text, runUiFrame, uiFocus, uiFocused } from "gnoblin/ui";
import type { PaintOp, UiKeyEvent } from "gnoblin/ui";

/** The compositor's answer to a password check: `result` is `ok`, or the wait in milliseconds under `rate-limited`. */
export interface LockAnswer {
    token: number;
    result: string;
    error: string;
}

/** The functions the shell host provides to the lock screen. A test attaches a fake one. */
export interface LockBackend {
    /** The session lock's state: `not-locked`, `pending` or `locked`. A tree with no lock node is `not-locked`. */
    state(): string;
    /** The lock node, which the screen is mounted under, or `undefined` when the tree has none. */
    node(): number | undefined;
    /** The width and height of the screen the lock screen covers, or `undefined` before the tree has one. */
    screen(): number[] | undefined;
    /** Records whether this screen is up. Returns the host's refusal code, if any. */
    engage(engaged: boolean): string | undefined;
    /** Asks the compositor to check the password. Returns the token the answer comes back under. */
    authenticate(password: string): number | undefined;
    takeAnswers(): LockAnswer[];
}

/** Everything that sets how the screen looks and what it says. */
export interface LockStyle {
    scrimFill: string;
    cardWidth: number;
    cardHeight: number;
    cardRadius: number;
    cardPadding: number;
    cardGap: number;
    cardFill: string;
    /** The card's distance from the top, as a share of the screen's height. */
    cardTop: number;
    clockSize: number;
    clockColor: string;
    entryHeight: number;
    entryRadius: number;
    entryFill: string;
    entrySize: number;
    entryColor: string;
    /** The most characters the entry takes. The compositor refuses a password over 1024 bytes. */
    entryLimit: number;
    messageSize: number;
    hintColor: string;
    errorColor: string;
    hint: string;
    checking: string;
    denied: string;
    busy: string;
    failed: string;
    /** The text of the rate-limit message; `{seconds}` is the wait, rounded up. */
    rateLimited: string;
}

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
    limit: number;
    size: number;
    color: string;
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
    before: number | undefined;
}

/**
 * The lock screen. `frame()` is called once a frame by the host: it takes the compositor's answer to a password check,
 * puts the screen up or takes it down by the lock's state, and ignores every key once the session is unlocked.
 */
export class LockScreen {
    readonly #backend: LockBackend;
    readonly style: LockStyle;
    #shown: Shown | null = null;
    /** The token of the check in flight. A second Enter while it runs is not a second attempt. */
    #pending: number | null = null;
    /** A password was accepted: nothing is drawn or heard until the state itself says `not-locked`. */
    #accepted = false;
    /** The generation is going away: the screen is down for good and never comes back. */
    #released = false;

    constructor(backend: LockBackend, style: LockStyle = defaultLockStyle) {
        this.#backend = backend;
        this.style = style;
    }

    /** Whether the screen is up. */
    get shown(): boolean {
        return this.#shown !== null;
    }

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
        }
    }

    /** Takes the screen down at once, as a generation that is going away does. */
    release(): void {
        this.#released = true;
        this.#hide();
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
        uiFocus(entry.node);
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
        if (held && shown.before !== undefined) {
            uiFocus(shown.before);
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
 */
export function runLockFrame(onError: (site: string, error: unknown) => void, after: () => void = () => {}): void {
    try {
        screen?.frame();
    } catch (error) {
        onError("LockScreen.frame", error);
    }
    runUiFrame();
    after();
}
