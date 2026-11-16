// Script-facing widgets that complement the Gnoblin UI toolkit: Entry, Clock and Bar.
// `EntryModel` keeps text editing separate from drawing, so the editing behaviour can be tested and reused.
// `Entry` reads key events from the canvas that has keyboard focus.

import { Box } from "gnoblin-std/ui/box";
import { uiNow, uiScreen } from "gnoblin-std/ui/session";
import { Text } from "gnoblin-std/ui/text";
import type { BoxOptions } from "gnoblin-std/ui/box";
import type { UiKeyEvent } from "gnoblin-std/ui/events";
import type { Color, PaintOp } from "gnoblin-std/ui/paint";
import { NEVER, onWake } from "gnoblin-core/wake";

// ---- EntryModel --------------------------------------------------------------------------------------------------

/** What a key types, with the control characters taken out: a Return, Tab or Escape types none of its own. */
function typed(text: string): string[] {
    return [...text].filter((char) => {
        const code = char.codePointAt(0) as number;
        return code >= 0x20 && code !== 0x7f && !(code >= 0x80 && code <= 0x9f);
    });
}

/**
 * The text, caret and selection for an entry, measured in characters (Unicode code points). `caret` marks where
 * typing goes; the selection spans `anchor` and `caret`, and is empty when they match.
 */
export class EntryModel {
    #chars: string[] = [];
    #caret = 0;
    #anchor = 0;

    /** Current entry text. Setting it moves the caret to the end and clears the selection. */
    get text(): string {
        return this.#chars.join("");
    }

    /** Replaces the text. The caret goes to the end and nothing is selected. */
    set text(value: string) {
        this.#chars = [...String(value)];
        this.#caret = this.#chars.length;
        this.#anchor = this.#caret;
    }

    /** Number of Unicode code points in the text. */
    get length(): number {
        return this.#chars.length;
    }

    /** Insertion point, counted in Unicode code points from the start of the text. */
    get caret(): number {
        return this.#caret;
    }

    /** Fixed end of an extended selection, in Unicode code points from the start. */
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

    /** Text in the current selection, or an empty string when no text is selected. */
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

    /** Moves the caret to the start of the text. With `extend`, keeps the selection anchor. */
    home(extend = false): void {
        this.move(0, extend);
    }

    /** Moves the caret to the end of the text. With `extend`, keeps the selection anchor. */
    end(extend = false): void {
        this.move(this.#chars.length, extend);
    }

    /** Selects all text. */
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

/** Options for an entry. Text, colour, font and selection appearance are optional. */
export interface EntryOptions extends BoxOptions {
    /** Initial text. */
    text?: string;
    /** Show a `*` for each character: a password. */
    secret?: boolean;
    /** Text size in logical pixels. */
    size?: number;
    /** Text colour. */
    color?: Color;
    /** Text font. */
    font?: string;
    /** Selection highlight colour. */
    selectionFill?: Color;
}

const SHIFT = new Set(["Shift_L", "Shift_R"]);
const CONTROL = new Set(["Control_L", "Control_R"]);

/**
 * A line of text you type into. Give it the keyboard (`entry.focus()`) and it reads the keys: letters type,
 * BackSpace and Delete remove, Left, Right, Home and End move the caret (with Shift held they select), Control+A
 * selects all, and Return calls `onSubmit` with the text. Each key it uses it stops, so a parent does not hear it
 * too; a subclass that wants one more key (Escape) calls `super.onKey(event)` first and then looks at `event.key`.
 *
 * The caret and the selection are drawn from the width the compositor measures for the text before them; until its
 * answer arrives a frame later, they are placed by the estimate the toolkit uses for any text.
 */
export class Entry extends Box {
    /** Editing state, with text and selection positions counted in Unicode code points. */
    readonly model = new EntryModel();
    /** Shows a `*` for each character instead of the text. */
    secret = false;
    /** Text size in logical pixels. */
    size = 14;
    /** Text colour. */
    color: Color = "#cdd6f4";
    /** Text font family. */
    font = "sans";
    /** Selection highlight colour. */
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

    /** Text being edited. Setting it moves the caret to the end and clears the selection. */
    get text(): string {
        return this.model.text;
    }

    /** Replaces the text and moves the caret to its end. */
    set text(value: string) {
        this.model.text = value;
    }

    /** What is drawn for `text`: the text itself, or a `*` for each character of a secret. */
    shown(text: string): string {
        return this.secret ? "*".repeat([...text].length) : text;
    }

    /** Updates the hidden measurement texts used to draw the entry. */
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

    /** Draws the text, selection highlight and focused caret. */
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

    /** Updates the model from a key event and stops keys handled by the entry. */
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
 * Formats a date using strftime-style fields: `%H` `%M` `%S` (24-hour time), `%I` and `%p` (12-hour), `%d` `%e`
 * `%m` `%y` `%Y` `%j` (date), `%a` `%A` `%b` `%B` (names), `%T` (`%H:%M:%S`), `%R` (`%H:%M`), `%F` (`%Y-%m-%d`)
 * and `%%`. Other `%` sequences remain unchanged. Uses local time.
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

/** Options for a clock widget. */
export interface ClockOptions extends BoxOptions {
    /** strftime-style format string. Default `%H:%M`. */
    format?: string;
    /** Text size in logical pixels. */
    size?: number;
    /** Text colour. */
    color?: Color;
    /** The time to show: the system clock by default. */
    now?: () => Date;
}

/** Clocks that are still alive; each updates when its displayed time changes. */
const clocks = new Set<Clock>();
onWake("clock widgets", () => {
    let at = NEVER;
    for (const clock of clocks) {
        if (!clock.alive) {
            clocks.delete(clock);
            continue;
        }
        at = Math.min(at, clock.changesAt());
    }
    return at;
});

/**
 * A text that shows the time, as `format` says (`%H:%M` by default). It follows the clock a frame at a time, and asks
 * the shell for a frame when its text is due to change: every 1000 ms for a format with the `%S` or `%T` field, every
 * 60000 ms otherwise, on the system clock's boundaries.
 */
export class Clock extends Text {
    /** Format string used to produce the clock text. Default `%H:%M`. */
    format = "%H:%M";
    /** Supplies the date to format. Defaults to the system clock. */
    now: () => Date = () => new Date();

    constructor(options: ClockOptions = {}) {
        super(options);
        if (options.format !== undefined) this.format = options.format;
        if (options.now !== undefined) this.now = options.now;
        this.text = formatTime(this.format, this.now());
        clocks.add(this);
    }

    /** Refreshes the displayed time using `format` and `now`. */
    override refresh(): void {
        this.text = formatTime(this.format, this.now());
    }

    /**
     * The reading of the toolkit's clock at which the text next differs. It counts from the system's time and not from
     * `now`, which is the script's and may throw: a failure in what asks the shell for its next frame must not end it.
     * A `now` that shows another time zone or a frozen time changes at the same minute boundaries or not at all, and a
     * frame too many is all that costs.
     */
    changesAt(): number {
        const period = /%[ST]/.test(this.format) ? 1000 : 60_000;
        return uiNow() + period - (((Date.now() % period) + period) % period);
    }
}

// ---- Bar ---------------------------------------------------------------------------------------------------------

/** Screen edge where a bar is placed. */
export type BarEdge = "top" | "bottom";

/** The output a bar sits on, as far as the bar needs it: its rectangle in the compositor's coordinates. */
export interface BarOutput {
    /** Whether the output is still connected. */
    readonly alive: boolean;
    /** Left edge in logical pixels. */
    readonly x: number;
    /** Top edge in logical pixels. */
    readonly y: number;
    /** Output width in logical pixels. */
    readonly width: number;
    /** Output height in logical pixels. */
    readonly height: number;
}

/**
 * Options for `Bar`. A bar is a strip along one screen edge. `hideWhen` lets it leave the screen when a fullscreen
 * window covers it.
 */
export interface BarOptions extends BoxOptions {
    /** Screen edge where the bar is placed. Default `top`. */
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
 * whole output, as the retired Lua shell's `Desk:sync_fullscreen` did. A bar's own `hideWhen` still applies. Null
 * stops it.
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
    /** Screen edge occupied by the bar. */
    edge: BarEdge = "top";
    /** Returns true to hide the bar and release its space. */
    hideWhen: (() => boolean) | null = null;
    /** Output used by the bar, or `null` to use the primary output. */
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

    /** Positions the bar on its output and updates its visibility. */
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
