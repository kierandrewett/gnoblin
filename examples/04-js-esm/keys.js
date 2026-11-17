// The launcher keys of example 4, a module imported by `shell.js` as `./keys.js`.
import { gnoblin } from "gnoblin-core";

/** The programs the keys start: each key runs one command. */
export const launchers = {
    "Super+Return": ["foot"],
    "Super+d": ["fuzzel"],
};

/** Binds every launcher on `input`. */
export function bindLaunchers(input) {
    for (const [chord, command] of Object.entries(launchers)) {
        input.bind(chord, () => {
            gnoblin.spawn(command);
        });
    }
}
