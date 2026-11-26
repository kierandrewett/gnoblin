// Example 7: a custom input binding written entirely in TypeScript.
//
//     gnoblin ./shell.ts
//
// A binding is a class: what a press of its chord does. This one nudges the focused window (or the first one when none has
// focus) by a fixed step. There is no Rust or C in this example; the shell is this file.
import { KeyBinding, gnoblin } from "gnoblin-core";
import type { BindingEvent } from "gnoblin-core";

const input = gnoblin.input.primary;

/** Moves a window `dx` and `dy` pixels when its chord is pressed. */
class NudgeBinding extends KeyBinding {
    constructor(
        private readonly dx: number,
        private readonly dy: number,
    ) {
        super();
    }

    override onPress(_event: BindingEvent): void | false {
        const target = input.focusedClient ?? gnoblin.clients.at(0);
        const bounds = target?.bounds;
        if (target === undefined || bounds === undefined) {
            return false;
        }
        const { x, y, width, height } = bounds;
        target.bounds = { x: x + this.dx, y: y + this.dy, width, height };
    }
}

input.bindBinding("Super+Right", new NudgeBinding(100, 0));
input.bindBinding("Super+Down", new NudgeBinding(0, 100));
