// The widgets of the Lua shell's `ui.lua` that the toolkit (`gnoblin/ui`) did not have: `Entry`, a line of text you type
// into, `Clock`, a text that shows the time, and `Bar`, a strip along the top or bottom of the screen that takes its
// room from the windows.
//
// `EntryModel` is the editing of an `Entry` without any drawing (the text, the caret, the selection), so it can be
// tested and reused. The keys an `Entry` reads are the ones the Lua `Entry` read: the `key` event of the canvas that
// holds the keyboard focus, with `key` (the keysym's name) and `text` (what it types).

import { Box, Text, uiScreen } from "gnoblin/ui";
import type { BoxOptions, Color, PaintOp, UiKeyEvent } from "gnoblin/ui";

// ---- EntryModel --------------------------------------------------------------------------------------------------

/** What a key types, with the control characters taken out: a Return, Tab or Escape types none of its own. */
function typed(text: string): string[] {
    return [...text].filter((char) => {
        const code = char.codePointAt(0) as number;
        return code >= 0x20 && code !== 0x7f && !(code >= 0x80 && code <= 0x9f);
    });
}

/**
 * The text of an entry, the caret and the selection, counted in characters (code points). `caret` is where typing
 * goes; the selection is what lies between `anchor` and `caret`, and is empty while they are the same place.
 */
export class EntryModel {
    #chars: string[] = [];
    #caret = 0;
    #anchor = 0;

    get text(): string {
        return this.#chars.join("");
    }

    /** Replaces the text. The caret goes to the end and nothing is selected. */
    set text(value: string) {
        this.#chars = [...String(value)];
        this.#caret = this.#chars.length;
        this.#anchor = this.#caret;
    }

    get length(): number {
        return this.#chars.length;
    }

    get caret(): number {
        return this.#caret;
    }

    get anchor(): number {
        return this.#anchor;
    }

    /** The selected range, or `null` while nothing is selected. */
    get selection(): { start: number; end: number } | null {
        if (this.#anchor === this.#caret) {
            return null;
        }
        return { start: Math.min(this.#anchor, this.#caret), end: Math.max(this.#anchor, this.#caret) };
    }

    get selectedText(): string {
        const range = this.selection;
        return range === null ? "" : this.#chars.slice(range.start, range.end).join("");
    }

    /** Types `text` over the selection, or at the caret. Control characters are left out. */
    insert(text: string): void {
        const chars = typed(text);
        if (chars.length === 0) {
            return;
        }
        this.#removeSelection();
        this.#chars.splice(this.#caret, 0, ...chars);
        this.#caret += chars.length;
        this.#anchor = this.#caret;
    }

    /** Deletes the selection, or the character before the caret. */
    backspace(): void {
        if (this.#removeSelection()) {
            return;
        }
        if (this.#caret > 0) {
            this.#chars.splice(this.#caret - 1, 1);
            this.#caret -= 1;
            this.#anchor = this.#caret;
        }
    }

    /** Deletes the selection, or the character after the caret. */
    delete(): void {
        if (this.#removeSelection()) {
            return;
        }
        if (this.#caret < this.#chars.length) {
            this.#chars.splice(this.#caret, 1);
        }
    }

    /** Moves the caret to `at` (clamped to the text). With `extend` the anchor stays, so the selection grows. */
    move(at: number, extend = false): void {
        this.#caret = Math.max(0, Math.min(this.#chars.length, Math.trunc(at)));
        if (!extend) {
            this.#anchor = this.#caret;
        }
    }

    /** One character left. A selection that is not extended collapses to its left edge instead of moving on. */
    left(extend = false): void {
        const range = this.selection;
        if (!extend && range !== null) {
            this.move(range.start);
        } else {
            this.move(this.#caret - 1, extend);
        }
    }

    /** One character right. A selection that is not extended collapses to its right edge. */
    right(extend = false): void {
        const range = this.selection;
        if (!extend && range !== null) {
            this.move(range.end);
        } else {
            this.move(this.#caret + 1, extend);
        }
    }

    home(extend = false): void {
        this.move(0, extend);
    }

    end(extend = false): void {
        this.move(this.#chars.length, extend);
    }

    selectAll(): void {
        this.#anchor = 0;
        this.#caret = this.#chars.length;
    }

    /** The characters of the text before `at`, as text. */
    before(at: number): string {
        return this.#chars.slice(0, Math.max(0, at)).join("");
    }

    #removeSelection(): boolean {
        const range = this.selection;
        if (range === null) {
            return false;
        }
        this.#chars.splice(range.start, range.end - range.start);
        this.#caret = range.start;
        this.#anchor = range.start;
        return true;
    }
}

// ---- Entry -------------------------------------------------------------------------------------------------------

export interface EntryOptions extends BoxOptions {
    text?: string;
    /** Show a `*` for each character: a password. */
    secret?: boolean;
    size?: number;
    color?: Color;
    font?: string;
    selectionFill?: Color;
}

const SHIFT = new Set(["Shift_L", "Shift_R"]);
const CONTROL = new Set(["Control_L", "Control_R"]);

/**
 * A line of text you type into. Give it the keyboard (`entry.focus()`) and it reads the keys: letters type, BackSpace and
 * Delete remove, Left, Right, Home and End move the caret (with Shift held they select), Control+A selects all, and
 * Return calls `onSubmit` with the text. Each key it uses it stops, so a parent does not hear it too; a subclass that
 * wants one more key (Escape) calls `super.onKey(event)` first and then looks at `event.key`.
 *
 * The caret and the selection are drawn from the width the compositor measures for the text before them; until its
 * answer arrives a frame later, they are placed by the estimate the toolkit uses for any text.
 */
export class Entry extends Box {
    readonly model = new EntryModel();
    secret = false;
    size = 14;
    color: Color = "#cdd6f4";
    font = "sans";
    selectionFill: Color = "#585b70";
    /** Called with the text when Return is pressed. */
    onSubmit?: (text: string) => void;
    readonly #line: Text;
    readonly #head: Text;
    readonly #mark: Text;
    #shift = false;
    #control = false;

    constructor(options: EntryOptions = {}) {
        super({ direction: "none", padding: 12, ...options });
        if (options.secret !== undefined) this.secret = options.secret;
        if (options.size !== undefined) this.size = options.size;
        if (options.color !== undefined) this.color = options.color;
        if (options.font !== undefined) this.font = options.font;
        if (options.selectionFill !== undefined) this.selectionFill = options.selectionFill;
        // Hidden texts the compositor measures: the whole line, what lies before the caret, and before the anchor.
        const probe = () => new Text({ visible: false });
        this.#line = probe();
        this.#head = probe();
        this.#mark = probe();
        this.add(this.#line);
        this.add(this.#head);
        this.add(this.#mark);
        if (options.text !== undefined) this.model.text = options.text;
    }

    get text(): string {
        return this.model.text;
    }

    set text(value: string) {
        this.model.text = value;
    }

    /** What is drawn for `text`: the text itself, or a `*` for each character of a secret. */
    shown(text: string): string {
        return this.secret ? "*".repeat([...text].length) : text;
    }

    override refresh(): void {
        for (const probe of [this.#line, this.#head, this.#mark]) {
            probe.size = this.size;
            probe.font = this.font;
        }
        const { model } = this;
        this.#line.text = this.shown(model.text);
        this.#head.text = this.shown(model.before(model.caret));
        this.#mark.text = this.shown(model.before(model.anchor));
    }

    override draw(): PaintOp[] {
        const ops = super.draw();
        const pad = this.padding;
        const line = this.#line.metrics();
        const top = Math.max(0, Math.floor((this.height - line.height) / 2));
        const caretX = pad + this.#head.metrics().width;
        if (this.model.selection !== null) {
            const markX = pad + this.#mark.metrics().width;
            const left = Math.min(caretX, markX);
            ops.push({
                op: "rect",
                x: left,
                y: top,
                width: Math.abs(caretX - markX),
                height: line.height,
                fill: this.selectionFill,
            });
        }
        if (this.model.length > 0) {
            ops.push({
                op: "text",
                text: this.shown(this.model.text),
                x: pad,
                y: top,
                size: this.size,
                color: this.color,
                font: this.font,
            });
        }
        if (this.focused) {
            ops.push({
                op: "line",
                x1: caretX,
                y1: top,
                x2: caretX,
                y2: top + line.height,
                width: 1.5,
                color: this.color,
            });
        }
        return ops;
    }

    override onKey(event: UiKeyEvent): void {
        const { key } = event;
        // A modifier's own events say whether Shift or Control is held for the keys that follow.
        if (SHIFT.has(key)) {
            this.#shift = event.pressed;
            return;
        }
        if (CONTROL.has(key)) {
            this.#control = event.pressed;
            return;
        }
        if (!event.pressed) {
            return; // the key coming up types nothing
        }
        const { model } = this;
        const extend = this.#shift;
        switch (key) {
            case "Return":
            case "KP_Enter":
                this.onSubmit?.(model.text);
                break;
            case "BackSpace":
                model.backspace();
                break;
            case "Delete":
            case "KP_Delete":
                model.delete();
                break;
            case "Left":
                model.left(extend);
                break;
            case "Right":
                model.right(extend);
                break;
            case "Home":
                model.home(extend);
                break;
            case "End":
                model.end(extend);
                break;
            default:
                if (this.#control && (key === "a" || key === "A")) {
                    model.selectAll();
                } else if (this.#control || event.text === "" || typed(event.text).length === 0) {
                    return;
                } else {
                    model.insert(event.text);
                }
        }
        event.stop();
    }
}

// ---- Clock -------------------------------------------------------------------------------------------------------

const DAYS = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
const MONTHS = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

function two(value: number): string {
    return String(value).padStart(2, "0");
}

/**
 * `date` as the strftime-style `format` writes it: `%H` `%M` `%S` (24-hour time), `%I` and `%p` (12-hour), `%d` `%e`
 * `%m` `%y` `%Y` `%j` (the date), `%a` `%A` `%b` `%B` (names), `%T` (`%H:%M:%S`), `%R` (`%H:%M`), `%F` (`%Y-%m-%d`)
 * and `%%`. Any other `%` sequence is left as it is. Local time.
 */
export function formatTime(format: string, date: Date): string {
    const hours12 = date.getHours() % 12 === 0 ? 12 : date.getHours() % 12;
    const dayOfYear = Math.floor(
        (Date.UTC(date.getFullYear(), date.getMonth(), date.getDate()) - Date.UTC(date.getFullYear(), 0, 0)) /
            86_400_000,
    );
    const fields: Record<string, string> = {
        H: two(date.getHours()),
        I: two(hours12),
        M: two(date.getMinutes()),
        S: two(date.getSeconds()),
        p: date.getHours() < 12 ? "AM" : "PM",
        d: two(date.getDate()),
        e: String(date.getDate()).padStart(2, " "),
        m: two(date.getMonth() + 1),
        y: two(date.getFullYear() % 100),
        Y: String(date.getFullYear()),
        j: String(dayOfYear).padStart(3, "0"),
        a: DAYS[date.getDay()].slice(0, 3),
        A: DAYS[date.getDay()],
        b: MONTHS[date.getMonth()].slice(0, 3),
        B: MONTHS[date.getMonth()],
        "%": "%",
    };
    fields.T = `${fields.H}:${fields.M}:${fields.S}`;
    fields.R = `${fields.H}:${fields.M}`;
    fields.F = `${fields.Y}-${fields.m}-${fields.d}`;
    return String(format).replace(/%(.)/gs, (whole, code: string) => fields[code] ?? whole);
}

export interface ClockOptions extends BoxOptions {
    format?: string;
    size?: number;
    color?: Color;
    /** The time to show: the system clock by default. */
    now?: () => Date;
}

/** A text that shows the time, as `format` says (`%H:%M` by default). It follows the clock a frame at a time. */
export class Clock extends Text {
    format = "%H:%M";
    now: () => Date = () => new Date();

    constructor(options: ClockOptions = {}) {
        super(options);
        if (options.format !== undefined) this.format = options.format;
        if (options.now !== undefined) this.now = options.now;
        this.text = formatTime(this.format, this.now());
    }

    override refresh(): void {
        this.text = formatTime(this.format, this.now());
    }
}

// ---- Bar ---------------------------------------------------------------------------------------------------------

export type BarEdge = "top" | "bottom";

/** The output a bar sits on, as far as the bar needs it: its rectangle in the compositor's coordinates. */
export interface BarOutput {
    readonly alive: boolean;
    readonly x: number;
    readonly y: number;
    readonly width: number;
    readonly height: number;
}

export interface BarOptions extends BoxOptions {
    edge?: BarEdge;
    /** When this returns true the bar is hidden and takes no room: a fullscreen window covers the screen. */
    hideWhen?: () => boolean;
    /** The output the bar sits on. Left out, the primary output; a bar whose output is unplugged is hidden. */
    output?: BarOutput | null;
}

/** The bars that are alive, in the order they were made. */
const bars: Bar[] = [];

let coveredBy: ((output: BarOutput | null) => boolean) | null = null;
let primaryOutput: (() => BarOutput | null) | null = null;

/**
 * Makes every bar give way (hidden, taking no room) while `covered` returns true for the output the bar sits on. The
 * shell passes "a fullscreen window is showing on the workspace that output shows", so a fullscreen window covers the
 * whole output, as the Lua shell's `Desk:sync_fullscreen` did. A bar's own `hideWhen` still applies. Null stops it.
 */
export function barsGiveWayTo(covered: ((output: BarOutput | null) => boolean) | null): void {
    coveredBy = covered;
}

/** Tells the bars which output is the primary one, the output a bar with no `output` of its own sits on. */
export function barsOnOutputs(primary: (() => BarOutput | null) | null): void {
    primaryOutput = primary;
}

/**
 * A strip along the top or bottom of an output (the primary one unless `output` names another), over the windows, that
 * takes its room from the windows there: the layouts place windows in what the bars leave (`barInsets`). It is a row
 * of boxes (`padding` 6, `gap` 12) as wide as the output. Bars on one edge of one output stack in the order they were
 * made. Make one and it is there; `destroy()` takes it away.
 */
export class Bar extends Box {
    edge: BarEdge = "top";
    hideWhen: (() => boolean) | null = null;
    output: BarOutput | null = null;

    constructor(options: BarOptions = {}) {
        super({ direction: "row", height: 32, fill: "#1e1e2e", padding: 6, gap: 12, ...options });
        if (options.edge !== undefined) this.edge = options.edge;
        if (options.hideWhen !== undefined) this.hideWhen = options.hideWhen;
        if (options.output !== undefined) this.output = options.output;
        bars.push(this);
        this.mountTop();
    }

    /** The output the bar sits on now: its own, or the primary one. Null while there is none. */
    get onOutput(): BarOutput | null {
        return this.output ?? primaryOutput?.() ?? null;
    }

    /** Whether the bar is on show, and so takes room. */
    get shown(): boolean {
        if (!this.alive) {
            return false;
        }
        const output = this.onOutput;
        if (this.output !== null && !output?.alive) {
            return false;
        }
        return !(this.hideWhen?.() ?? false) && !(coveredBy?.(output) ?? false);
    }

    override refresh(): void {
        this.visible = this.shown;
        const output = this.onOutput;
        const place = output?.alive
            ? { x: output.x, y: output.y, width: output.width, height: output.height }
            : this.output === null
              ? uiScreen()
              : null;
        if (place === null) {
            return;
        }
        const origin = "x" in place ? place : { ...place, x: 0, y: 0 };
        this.width = origin.width;
        let before = 0;
        for (const other of liveBars()) {
            if (other === this) {
                break;
            }
            if (other.edge === this.edge && other.onOutput === output && other.shown) {
                before += other.height;
            }
        }
        this.x = origin.x;
        this.y = this.edge === "top" ? origin.y + before : origin.y + origin.height - before - this.height;
    }
}

function liveBars(): Bar[] {
    for (let at = bars.length - 1; at >= 0; at--) {
        if (!bars[at].alive) {
            bars.splice(at, 1);
        }
    }
    return bars;
}

/**
 * The room the bars take from the top and the bottom of `output` (the primary output when none is given). Windows are
 * laid out in what is left.
 */
export function barInsets(output?: BarOutput | null): { top: number; bottom: number } {
    const on = output ?? primaryOutput?.() ?? null;
    let top = 0;
    let bottom = 0;
    for (const bar of liveBars()) {
        if (!bar.shown || bar.onOutput !== on) {
            continue;
        }
        if (bar.edge === "top") {
            top += bar.height;
        } else {
            bottom += bar.height;
        }
    }
    return { top, bottom };
}
