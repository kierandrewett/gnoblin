import { Box } from "gnoblin-std/ui/box";
import { Text } from "gnoblin-std/ui/text";
import type { BoxOptions } from "gnoblin-std/ui/box";
import type { UiKeyEvent } from "gnoblin-std/ui/events";
import type { Color, PaintOp } from "gnoblin-std/ui/paint";
/**
 * The text, caret and selection for an entry, measured in characters (Unicode code points). `caret` marks where
 * typing goes; the selection spans `anchor` and `caret`, and is empty when they match.
 */
export declare class EntryModel {
    #private;
    /** Current entry text. Setting it moves the caret to the end and clears the selection. */
    get text(): string;
    /** Replaces the text. The caret goes to the end and nothing is selected. */
    set text(value: string);
    /** Number of Unicode code points in the text. */
    get length(): number;
    /** Insertion point, counted in Unicode code points from the start of the text. */
    get caret(): number;
    /** Fixed end of an extended selection, in Unicode code points from the start. */
    get anchor(): number;
    /** The selected range, or `null` while nothing is selected. */
    get selection(): {
        start: number;
        end: number;
    } | null;
    /** Text in the current selection, or an empty string when no text is selected. */
    get selectedText(): string;
    /** Types `text` over the selection, or at the caret. Control characters are left out. */
    insert(text: string): void;
    /** Deletes the selection, or the character before the caret. */
    backspace(): void;
    /** Deletes the selection, or the character after the caret. */
    delete(): void;
    /** Moves the caret to `at` (clamped to the text). With `extend` the anchor stays, so the selection grows. */
    move(at: number, extend?: boolean): void;
    /** One character left. A selection that is not extended collapses to its left edge instead of moving on. */
    left(extend?: boolean): void;
    /** One character right. A selection that is not extended collapses to its right edge. */
    right(extend?: boolean): void;
    /** Moves the caret to the start of the text. With `extend`, keeps the selection anchor. */
    home(extend?: boolean): void;
    /** Moves the caret to the end of the text. With `extend`, keeps the selection anchor. */
    end(extend?: boolean): void;
    /** Selects all text. */
    selectAll(): void;
    /** The characters of the text before `at`, as text. */
    before(at: number): string;
}
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
/**
 * A line of text you type into. Give it the keyboard (`entry.focus()`) and it reads the keys: letters type,
 * BackSpace and Delete remove, Left, Right, Home and End move the caret (with Shift held they select), Control+A
 * selects all, and Return calls `onSubmit` with the text. Each key it uses it stops, so a parent does not hear it
 * too; a subclass that wants one more key (Escape) calls `super.onKey(event)` first and then looks at `event.key`.
 *
 * The caret and the selection are drawn from the width the compositor measures for the text before them; until its
 * answer arrives a frame later, they are placed by the estimate the toolkit uses for any text.
 */
export declare class Entry extends Box {
    #private;
    /** Editing state, with text and selection positions counted in Unicode code points. */
    readonly model: EntryModel;
    /** Shows a `*` for each character instead of the text. */
    secret: boolean;
    /** Text size in logical pixels. */
    size: number;
    /** Text colour. */
    color: Color;
    /** Text font family. */
    font: string;
    /** Selection highlight colour. */
    selectionFill: Color;
    /** Called with the text when Return is pressed. */
    onSubmit?: (text: string) => void;
    constructor(options?: EntryOptions);
    /** Text being edited. Setting it moves the caret to the end and clears the selection. */
    get text(): string;
    /** Replaces the text and moves the caret to its end. */
    set text(value: string);
    /** What is drawn for `text`: the text itself, or a `*` for each character of a secret. */
    shown(text: string): string;
    /** Updates the hidden measurement texts used to draw the entry. */
    refresh(): void;
    /** Draws the text, selection highlight and focused caret. */
    draw(): PaintOp[];
    /** Updates the model from a key event and stops keys handled by the entry. */
    onKey(event: UiKeyEvent): void;
}
/**
 * Formats a date using strftime-style fields: `%H` `%M` `%S` (24-hour time), `%I` and `%p` (12-hour), `%d` `%e`
 * `%m` `%y` `%Y` `%j` (date), `%a` `%A` `%b` `%B` (names), `%T` (`%H:%M:%S`), `%R` (`%H:%M`), `%F` (`%Y-%m-%d`)
 * and `%%`. Other `%` sequences remain unchanged. Uses local time.
 */
export declare function formatTime(format: string, date: Date): string;
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
/**
 * A text that shows the time, as `format` says (`%H:%M` by default). It follows the clock a frame at a time, and asks
 * the shell for a frame when its text is due to change: every 1000 ms for a format with the `%S` or `%T` field, every
 * 60000 ms otherwise, on the system clock's boundaries.
 */
export declare class Clock extends Text {
    /** Format string used to produce the clock text. Default `%H:%M`. */
    format: string;
    /** Supplies the date to format. Defaults to the system clock. */
    now: () => Date;
    constructor(options?: ClockOptions);
    /** Refreshes the displayed time using `format` and `now`. */
    refresh(): void;
    /**
     * The reading of the toolkit's clock at which the text next differs. It counts from the system's time and not from
     * `now`, which is the script's and may throw: a failure in what asks the shell for its next frame must not end it.
     * A `now` that shows another time zone or a frozen time changes at the same minute boundaries or not at all, and a
     * frame too many is all that costs.
     */
    changesAt(): number;
}
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
/**
 * Makes every bar give way (hidden, taking no room) while `covered` returns true for the output the bar sits on. The
 * shell passes "a fullscreen window is showing on the workspace that output shows", so a fullscreen window covers the
 * whole output, as the retired Lua shell's `Desk:sync_fullscreen` did. A bar's own `hideWhen` still applies. Null
 * stops it.
 */
export declare function barsGiveWayTo(covered: ((output: BarOutput | null) => boolean) | null): void;
/** Tells the bars which output is the primary one, the output a bar with no `output` of its own sits on. */
export declare function barsOnOutputs(primary: (() => BarOutput | null) | null): void;
/**
 * A strip along the top or bottom of an output (the primary one unless `output` names another), over the windows, that
 * takes its room from the windows there: the layouts place windows in what the bars leave (`barInsets`). It is a row
 * of boxes (`padding` 6, `gap` 12) as wide as the output. Bars on one edge of one output stack in the order they were
 * made. Make one and it is there; `destroy()` takes it away.
 */
export declare class Bar extends Box {
    /** Screen edge occupied by the bar. */
    edge: BarEdge;
    /** Returns true to hide the bar and release its space. */
    hideWhen: (() => boolean) | null;
    /** Output used by the bar, or `null` to use the primary output. */
    output: BarOutput | null;
    constructor(options?: BarOptions);
    /** The output the bar sits on now: its own, or the primary one. Null while there is none. */
    get onOutput(): BarOutput | null;
    /** Whether the bar is on show, and so takes room. */
    get shown(): boolean;
    /** Positions the bar on its output and updates its visibility. */
    refresh(): void;
}
/**
 * The room the bars take from the top and the bottom of `output` (the primary output when none is given). Windows are
 * laid out in what is left.
 */
export declare function barInsets(output?: BarOutput | null): {
    top: number;
    bottom: number;
};
