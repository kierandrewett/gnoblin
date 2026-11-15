// Moving through a menu by keyboard: which row is hot, and what each key does.

/** What the navigator needs of a row. */
export interface NavRow {
    readonly label: string;
    readonly enabled: boolean;
    readonly submenu: boolean;
}

/**
 * Which row of a menu the keyboard is on, over the rows that can be run: a card of rows and, once one is opened, a
 * submenu's. Selection is kept per card, so closing the submenu leaves its parent row selected.
 */
export class MenuNavigator<R extends NavRow = NavRow> {
    readonly rows: readonly R[];
    #sub: readonly R[] = [];
    #depth: 1 | 2 = 1;
    #picked: [number, number] = [-1, -1];

    constructor(rows: readonly R[]) {
        this.rows = rows;
    }

    get depth(): 1 | 2 {
        return this.#depth;
    }

    get subRows(): readonly R[] {
        return this.#sub;
    }

    /** The rows the keyboard is in. */
    active(): readonly R[] {
        return this.#depth === 2 ? this.#sub : this.rows;
    }

    /** The place of the selection among the active rows, or -1. */
    get index(): number {
        return this.#picked[this.#depth - 1];
    }

    /** The row the keyboard is on, or null. */
    selected(): R | null {
        return this.active()[this.index] ?? null;
    }

    /** Whether `row` is the one picked in its card. */
    isSelected(row: R): boolean {
        return this.rows[this.#picked[0]] === row || this.#sub[this.#picked[1]] === row;
    }

    /** Puts the selection on `row`. The pointer going back to the first card closes the submenu it left. */
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

    home(): void {
        this.#picked[this.#depth - 1] = -1;
        this.move(1);
    }

    end(): void {
        this.#picked[this.#depth - 1] = -1;
        this.move(-1);
    }

    /** The first row after the selection, wrapping, whose label starts with `letter` (compared without case). */
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

    /** Opens a submenu of `rows` and puts the selection on its first row that can be run. */
    openSubmenu(rows: readonly R[]): void {
        this.closeSubmenu();
        this.#sub = rows;
        this.#depth = 2;
        this.#picked[1] = -1;
        this.move(1);
    }

    closeSubmenu(): void {
        if (this.#depth !== 2) {
            return;
        }
        this.#sub = [];
        this.#depth = 1;
        this.#picked[1] = -1;
    }
}

/** What a key does to a menu. */
export type MenuKeyAction =
    | { do: "move"; step: 1 | -1 }
    | { do: "home" | "end" | "activate" | "right" | "left" | "escape" }
    | { do: "jump"; letter: string };

/** The action for a key of the keyboard while a menu is open, or `null` for a key the menu does not use. */
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
