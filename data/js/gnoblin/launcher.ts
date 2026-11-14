// The stock launcher (data/retired shell/gnoblin/shell/launcher.retired shell): Super+Space opens a command entry on the screen. Return starts
// the command and closes the entry; Escape closes it. It is an `Entry` that spawns what was typed. A config that gives
// `launcher` a command line runs that program on Super+Space instead and has no entry.

import { uiScreen } from "gnoblin/ui";
import type { UiKeyEvent } from "gnoblin/ui";
import { Entry } from "gnoblin/widgets";
import type { EntryOptions } from "gnoblin/widgets";

/** The host module's global, which is there before any stock module runs. */
declare const gnoblin: { spawn(command: string): void };

export interface LauncherOptions extends EntryOptions {
    /** Starts the typed command. Default: `gnoblin.spawn`. */
    spawn?: (command: string) => void;
}

/** A command entry, 600 by 48. */
export class Launcher extends Entry {
    readonly #spawn: (command: string) => void;

    constructor(options: LauncherOptions = {}) {
        super({ width: 600, height: 48, radius: 12, fill: "#313244", size: 18, ...options });
        this.#spawn = options.spawn ?? ((command) => gnoblin.spawn(command));
        this.onSubmit = (text) => {
            if (text.trim() !== "") {
                this.#spawn(text);
            }
            this.destroy();
        };
    }

    // Entry's typing runs first. Escape is the launcher's own key; Return was Entry's submit above.
    override onKey(event: UiKeyEvent): void {
        super.onKey(event);
        if (event.pressed && event.key === "Escape") {
            this.destroy();
            event.stop();
        }
    }
}

let current: Launcher | null = null;

/** Opens the launcher in the middle of the screen's width, over the windows, and gives it the keyboard. One at a time. */
export function openLauncher(options: LauncherOptions = {}): Launcher {
    if (current !== null && current.alive) {
        current.focus();
        return current;
    }
    const screen = uiScreen();
    const launcher = new Launcher({ x: Math.floor((screen?.width ?? 1200) / 2) - 300, y: 200, ...options });
    launcher.mountTop();
    launcher.focus();
    current = launcher;
    return launcher;
}
