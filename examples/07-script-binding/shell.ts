// Example 7: a custom input binding written entirely in TypeScript.
//
//     gnoblin ./shell.ts
//
// A binding is a class: the trigger it listens for and what a press does. This one nudges the focused window (or the first one
// when none has focus) by a fixed step. There is no Rust or C in this example; the shell is this file.
import { KeyBinding, gnoblin } from "gnoblin-core";
import type { BindingEvent, Client } from "gnoblin-core";

/** Moves a window `dx` and `dy` pixels when its chord is pressed. */
class NudgeBinding extends KeyBinding {
    constructor(
        id: string,
        trigger: string,
        private readonly dx: number,
        private readonly dy: number,
    ) {
        super({ id, trigger });
    }

    override onPress(_event: BindingEvent): void | false {
        const target: Client | undefined = gnoblin.input.primary.focusedClient ?? [...gnoblin.clients][0];
        if (target === undefined) {
            return false;
        }
        const { x, y, width, height } = target.bounds;
        target.bounds = { x: x + this.dx, y: y + this.dy, width, height };
    }
}

const input = gnoblin.input.primary;
input.bind("Super+Right", new NudgeBinding("nudge-right", "Super+Right", 100, 0));
input.bind("Super+Down", new NudgeBinding("nudge-down", "Super+Down", 0, 100));
