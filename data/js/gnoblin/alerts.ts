// The stock "not responding" alert, ported from data/retired shell/gnoblin/shell/alerts.retired shell.
//
// The compositor pings every client that has a window. One that leaves a ping unanswered for longer than
// `window-management.check-alive-timeout` (5000 ms by default; 0 turns the pings off) has its windows marked
// `unresponsive`, and this module asks the user what to do about each:
//   * Wait, the default: the alert closes, and does not come back for that window until it has answered a ping and then
//     stopped answering again.
//   * Force quit: the window's process is killed (`Client.kill()`). The compositor refuses its own process, init and a
//     process that is not the one that opened the window.
//
// Escape is Wait, Tab, Left and Right move between the buttons, Return or Space presses the focused one, and Force quit
// does nothing for the first `armSeconds`, so a click or a key already on its way when the alert appears cannot press it.
// It is `secure` when the window is, since it shows the window's title. It closes by itself when the window answers
// again, closes, or the session locks, and while the session is locked no alert opens. One alert is open for each window
// that is unresponsive. It takes the keyboard, because an unresponsive window is the one the user was typing into, and
// gives it back when it closes. It closes an open window menu first.
//
// `AlertTracker` is the state machine alone (which windows get an alert, which are waited on, what an answer does), so
// it is tested without a compositor. Turn the alert off with `defineConfig({ alerts: false })`.

import { Box, Text, onUiFrame, uiFocus, uiFocused, uiLocked, uiNow, uiScreen } from "gnoblin/ui";
import type { UiKeyEvent, UiPointerEvent } from "gnoblin/ui";

const LEFT = 272;
const WIDTH = 480;
const PAD = 20;
const GAP = 10;
const BUTTON_H = 44;

// ---- the state machine -------------------------------------------------------------------------------------------

/** What an alert needs of a window. A `Client` of the host module is one. */
export interface AlertClient {
    readonly alive: boolean;
    readonly unresponsive: boolean;
    readonly title: string;
    readonly secure?: boolean;
    kill(): void;
}

/** What the user chose, or `recovered` when the alert ended without them. */
export type AlertAnswer = "wait" | "force" | "recovered";

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
export class AlertTracker<W extends AlertClient> {
    /** The windows whose user chose Wait or Force quit: no alert for them until they have answered a ping. */
    readonly #waited = new Set<W>();
    readonly #open = new Set<W>();

    /** Whether an alert is open for `window`. */
    isOpen(window: W): boolean {
        return this.#open.has(window);
    }

    isWaiting(window: W): boolean {
        return this.#waited.has(window);
    }

    check(windows: Iterable<W>, locked: boolean): AlertChanges<W> {
        const changes: AlertChanges<W> = { open: [], close: [] };
        const live = new Set<W>();
        for (const window of windows) {
            if (!window.alive) {
                this.#waited.delete(window);
                continue;
            }
            live.add(window);
            if (window.unresponsive) {
                if (!this.#waited.has(window) && !this.#open.has(window) && !locked) {
                    this.#open.add(window);
                    changes.open.push(window);
                }
            } else {
                this.#waited.delete(window);
                if (this.#open.has(window)) {
                    changes.close.push(window);
                }
            }
        }
        for (const window of this.#open) {
            if ((locked || !live.has(window)) && !changes.close.includes(window)) {
                changes.close.push(window);
            }
        }
        return changes;
    }

    /**
     * The alert for `window` ended with `answer`. Returns whether the window is to be killed. A window the user chose
     * Wait or Force quit for is waited on until it answers a ping again.
     */
    answered(window: W, answer: AlertAnswer): boolean {
        this.#open.delete(window);
        if (answer === "recovered") {
            return false;
        }
        this.#waited.add(window);
        return answer === "force";
    }
}

/** Text from a peer is for display only: no control characters, no lone surrogates, and no more than `max` characters. */
export function cleanText(text: string, max: number): string {
    const raw = String(text ?? "");
    if (/[\ud800-\udbff](?![\udc00-\udfff])|(?<![\ud800-\udbff])[\udc00-\udfff]/.test(raw)) {
        return "(unreadable)";
    }
    const plain = raw.replace(/[\u0000-\u001f\u007f-\u009f]/g, " ");
    const chars = [...plain];
    return chars.length > max ? `${chars.slice(0, max).join("")}\u2026` : plain;
}

// ---- the card ----------------------------------------------------------------------------------------------------

/** One of the two buttons: Wait or Force quit. A button that is not enabled is dim and does nothing. */
class AlertButton extends Box {
    readonly label: Text;
    enabled = true;
    /** The button the keyboard is on. */
    picked = false;
    readonly #dialog: NotRespondingDialog;
    readonly #index: number;

    constructor(dialog: NotRespondingDialog, index: number, label: string) {
        super({ radius: 10, height: BUTTON_H, grow: 1 });
        this.#dialog = dialog;
        this.#index = index;
        this.label = new Text({ text: label });
        this.add(this.label);
    }

    override refresh(): void {
        if (!this.enabled) {
            this.fill = "#2a2a3a";
            this.label.color = "#6c7086";
        } else {
            this.fill = this.picked ? "#89b4fa" : "#45475a";
            this.label.color = this.picked ? "#11111b" : "#cdd6f4";
        }
    }

    override arrange(): void {
        this.label.x = Math.floor((this.width - this.label.width) / 2);
        this.label.y = Math.floor((this.height - this.label.height) / 2);
    }

    override onPress(event: UiPointerEvent): void {
        event.stop();
        if (event.button === LEFT) {
            this.#dialog.select(this.#index);
            this.#dialog.activate(this.#index);
        }
    }
}

/**
 * The alert: what is wrong, and Wait and Force quit. `onChosen` is called once, with the answer. Force quit is off until
 * `arm()`, which the controller calls `armSeconds` after the alert opened.
 */
export class NotRespondingDialog extends Box {
    readonly choices: AlertButton[];
    index = 0;
    armed = false;
    onChosen?: (answer: "wait" | "force") => void;

    constructor(name: string, secure: boolean, screen: { width: number; height: number }) {
        const rows = [
            new Text({ text: cleanText(`${name} is not responding`, 80), size: 17, color: "#cdd6f4", height: 26 }),
            new Text({ text: "You can wait for it, or force it to quit.", size: 14, color: "#cdd6f4", height: 20 }),
            new Text({ text: "Anything it has not saved will be lost.", size: 14, color: "#a6adc8", height: 20 }),
        ];
        const height = rows.reduce((total, row) => total + row.height + GAP, 0);
        const total = 2 * PAD + height + BUTTON_H;
        super({
            x: Math.floor((screen.width - WIDTH) / 2),
            y: Math.max(0, Math.floor((screen.height - total) / 4)),
            width: WIDTH,
            height: total,
            radius: 16,
            fill: "#1e1e2e",
            padding: PAD,
            gap: GAP,
            direction: "column",
            secure,
        });
        for (const row of rows) {
            this.add(row);
        }
        const buttons = new Box({ direction: "row", gap: 12, height: BUTTON_H });
        this.choices = [new AlertButton(this, 0, "Wait"), new AlertButton(this, 1, "Force quit")];
        this.choices[1].enabled = false; // until the alert is armed
        for (const button of this.choices) {
            buttons.add(button);
        }
        this.add(buttons);
        this.select(0); // Wait is the default
    }

    /** The button the keyboard presses. */
    select(index: number): void {
        this.index = index;
        this.choices.forEach((button, at) => {
            button.picked = at === index;
        });
    }

    /** Lets Force quit be pressed. */
    arm(): void {
        this.armed = true;
        this.choices[1].enabled = true;
    }

    activate(index: number): void {
        if (index === 1 && !this.armed) {
            return;
        }
        this.onChosen?.(index === 0 ? "wait" : "force");
    }

    override onPress(event: UiPointerEvent): void {
        event.stop();
        this.focus();
    }

    override onKey(event: UiKeyEvent): void {
        if (!event.pressed) {
            return;
        }
        switch (event.key) {
            case "Escape":
                this.onChosen?.("wait");
                break;
            case "Tab":
            case "ISO_Left_Tab":
            case "Left":
            case "Right":
                this.select((this.index + 1) % 2);
                break;
            case "Return":
            case "KP_Enter":
            case "space":
                this.activate(this.index);
                break;
            default:
                return;
        }
        event.stop();
    }
}

// ---- the controller ----------------------------------------------------------------------------------------------

/** What the controller needs of the shell around it. */
export interface AlertWorld {
    /** Every window. */
    clients(): Iterable<AlertClient>;
    /** Closes an open window menu: an alert takes the keyboard from it. */
    closeMenu?(): void;
}

export interface AlertOptions {
    /** Seconds Force quit ignores a press after the alert opens. Default 0.5. */
    armSeconds?: number;
    /** Milliseconds between looks at the windows. Default 250: the property is read, not signalled. */
    intervalMs?: number;
}

interface Card {
    dialog: NotRespondingDialog;
    /** The node that held the keyboard before the alert took it. */
    before: number | null;
    armAt: number;
}

/**
 * Looks at the windows a few times a second, opens the alert for one that has just stopped answering and closes the alert
 * of one that answers again, has gone, or lies under a lock. `unresponsive` is a property the compositor writes, so the
 * alert reads it: it does not wait for a signal there is none of.
 */
export class NotRespondingAlerts {
    armSeconds = 0.5;
    intervalMs = 250;
    readonly tracker = new AlertTracker<AlertClient>();
    readonly #cards = new Map<AlertClient, Card>();
    #world: AlertWorld | null = null;
    #lastLook = Number.NEGATIVE_INFINITY;

    get started(): boolean {
        return this.#world !== null;
    }

    /** The alert open for `client`, or `undefined`. */
    cardOf(client: AlertClient): NotRespondingDialog | undefined {
        return this.#cards.get(client)?.dialog;
    }

    start(world: AlertWorld, options: AlertOptions = {}): void {
        this.#world = world;
        if (options.armSeconds !== undefined) this.armSeconds = options.armSeconds;
        if (options.intervalMs !== undefined) this.intervalMs = options.intervalMs;
        onUiFrame(() => this.#frame());
    }

    /** One look at the windows, now. */
    check(): void {
        const world = this.#world;
        const screen = uiScreen();
        if (world === null) {
            return;
        }
        const changes = this.tracker.check(world.clients() as Iterable<AlertClient>, uiLocked());
        for (const client of changes.close) {
            this.#finish(client, "recovered");
        }
        // No screen, no card: the window is looked at again once there is one.
        for (const client of changes.open) {
            if (screen === null) {
                this.tracker.answered(client, "recovered");
            } else {
                this.#show(client, screen);
            }
        }
    }

    #frame(): void {
        const now = uiNow();
        for (const [, card] of this.#cards) {
            if (!card.dialog.armed && now >= card.armAt) {
                card.dialog.arm();
            }
        }
        if (now - this.#lastLook >= this.intervalMs) {
            this.#lastLook = now;
            this.check();
        }
    }

    #show(client: AlertClient, screen: { width: number; height: number }): void {
        this.#world?.closeMenu?.();
        const dialog = new NotRespondingDialog(cleanTitle(client.title), client.secure === true, screen);
        const card: Card = { dialog, before: uiFocused(), armAt: uiNow() + this.armSeconds * 1000 };
        this.#cards.set(client, card);
        dialog.onChosen = (answer) => this.#finish(client, answer);
        dialog.mountTop();
        dialog.focus();
    }

    /** Ends the alert for `client`: the card goes, the keyboard goes back, and a Force quit kills. */
    #finish(client: AlertClient, answer: AlertAnswer): void {
        const card = this.#cards.get(client);
        if (card === undefined) {
            return;
        }
        this.#cards.delete(client);
        card.dialog.destroy();
        try {
            uiFocus(card.before);
        } catch {
            uiFocus(null);
        }
        if (this.tracker.answered(client, answer)) {
            client.kill();
        }
    }
}

/** What a window is called in its alert: its title as text, or "This application" when it has none. */
function cleanTitle(title: string): string {
    const name = cleanText(title, 60);
    return name === "" ? "This application" : name;
}

/** The stock alert. Started by `defineConfig` unless the config says `alerts: false`. */
export const notRespondingAlerts = new NotRespondingAlerts();
