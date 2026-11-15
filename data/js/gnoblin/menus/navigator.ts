// Keyboard selection and key mapping for menu rows.

/** The fields the navigator uses to select a row. */
export interface NavRow {
    /** Text shown for the row and used for letter-key navigation. */
    readonly label: string;
    /** Whether the row can be selected and activated. */
    readonly enabled: boolean;
    /** Whether the row opens a submenu. */
    readonly submenu: boolean;
}

/**
 * Tracks keyboard selection across a menu and its open submenu. Each card keeps its selection when the submenu closes.
 */
export class MenuNavigator<R extends NavRow = NavRow> {
    /** Rows in the main card. */
    readonly rows: readonly R[];
    #sub: readonly R[] = [];
    #depth: 1 | 2 = 1;
    #picked: [number, number] = [-1, -1];

    constructor(rows: readonly R[]) {
        this.rows = rows;
    }

    /** Whether selection is in the main card (1) or submenu (2). */
    get depth(): 1 | 2 {
        return this.#depth;
    }

    /** Rows in the open submenu, or an empty list when it is closed. */
    get subRows(): readonly R[] {
        return this.#sub;
    }

    /** The rows in the active card. */
    active(): readonly R[] {
        return this.#depth === 2 ? this.#sub : this.rows;
    }

    /** The selected row's index in the active card, or -1. */
    get index(): number {
        return this.#picked[this.#depth - 1];
    }

    /** The row the keyboard is on, or null. */
    selected(): R | null {
        return this.active()[this.index] ?? null;
    }

    /** Returns whether `row` is selected in either card. */
    isSelected(row: R): boolean {
        return this.rows[this.#picked[0]] === row || this.#sub[this.#picked[1]] === row;
    }

    /** Selects `row`; selecting a row outside the submenu closes it. */
    select(row: R): void {
        if (this.#depth === 2 && !this.#sub.includes(row)) {
            this.closeSubmenu();
        }
        const at = this.active().indexOf(row);
        if (at >= 0) {
            this.#picked[this.#depth - 1] = at;
        }
    }

    /**
     * Moves the selection by `step` over the rows that can be run, wrapping at the ends. With nothing selected a step
     * forward lands on the first row and a step back on the last.
     */
    move(step: 1 | -1): void {
        const rows = this.active();
        const count = rows.length;
        let at = this.index;
        if (at < 0) {
            at = step > 0 ? -1 : count;
        }
        for (let tried = 0; tried < count; tried++) {
            at = (((at + step) % count) + count) % count;
            if (rows[at].enabled) {
                this.#picked[this.#depth - 1] = at;
                return;
            }
        }
    }

    /** Selects the first enabled row. */
    home(): void {
        this.#picked[this.#depth - 1] = -1;
        this.move(1);
    }

    /** Selects the last enabled row. */
    end(): void {
        this.#picked[this.#depth - 1] = -1;
        this.move(-1);
    }

    /** Selects the next enabled row whose label starts with `letter`, ignoring case. */
    jump(letter: string): void {
        const rows = this.active();
        const count = rows.length;
        for (let step = 1; step <= count; step++) {
            const at = (this.index + step + count) % count;
            if (rows[at].enabled && rows[at].label.slice(0, 1).toLowerCase() === letter.toLowerCase()) {
                this.#picked[this.#depth - 1] = at;
                return;
            }
        }
    }

    /** Opens a submenu and selects its first enabled row. */
    openSubmenu(rows: readonly R[]): void {
        this.closeSubmenu();
        this.#sub = rows;
        this.#depth = 2;
        this.#picked[1] = -1;
        this.move(1);
    }

    /** Closes the submenu and restores the parent card's selection. */
    closeSubmenu(): void {
        if (this.#depth !== 2) {
            return;
        }
        this.#sub = [];
        this.#depth = 1;
        this.#picked[1] = -1;
    }
}

/** An action produced by a menu key. */
export type MenuKeyAction =
    | { do: "move"; step: 1 | -1 }
    | { do: "home" | "end" | "activate" | "right" | "left" | "escape" }
    | { do: "jump"; letter: string };

/** Maps a key to its menu action, or returns `null` when the menu does not use it. */
export function menuKeyAction(key: string, text: string): MenuKeyAction | null {
    switch (key) {
        case "Escape":
            return { do: "escape" };
        case "Down":
        case "Tab":
            return { do: "move", step: 1 };
        case "Up":
        case "ISO_Left_Tab":
            return { do: "move", step: -1 };
        case "Home":
            return { do: "home" };
        case "End":
            return { do: "end" };
        case "Return":
        case "KP_Enter":
        case "space":
            return { do: "activate" };
        case "Right":
            return { do: "right" };
        case "Left":
            return { do: "left" };
        default:
            return [...text].length === 1 && /\S/.test(text) ? { do: "jump", letter: text } : null;
    }
}
