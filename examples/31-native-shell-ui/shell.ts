// Example 31 in TypeScript: the parts of the stock shell above the toolkit, from libgnoblin-std.
//
//     gnoblin ./shell.ts
//
// The classes are generated from the catalog the library serves, so `gnoblin-std/generated/objects` has one for each of them. This shell:
//
// - starts the not-responding alert, which looks at the windows and opens a card for one that stops answering the compositor's pings: Escape
//   waits, Tab moves to Force quit, and Force quit takes a press only after `armMs`;
// - puts the stock lock screen to work: a clock and a password entry over the screen while the session is locked;
// - opens the launcher with `Super+space`: Return runs the command typed and closes it, Escape closes it;
// - flips the overview with `Super+o`.
//
// Every line it logs starts with `shell-ui:`, spelt as the Rust shell of the same example (`31-native-shell-ui`) spells it, so that one test
// reads all three.
import { gnoblin } from "gnoblin-core";
import { Launcher, LockScreen, NotRespondingAlerts, Overview } from "gnoblin-std/generated/objects";
import type {} from "gnoblin-std";

// A long delay, so that a Return pressed at once finds Force quit still off.
const alerts = NotRespondingAlerts.get();
alerts.armMs = 3000;
alerts.on("answered", (client) => console.log(`shell-ui: the alert for window ${client} ended`));
alerts.start();

// Making the screen puts it to work for the generation: it is up while the session is locked.
const lock = new LockScreen(LockScreen.defaultLockStyle());
console.log(`shell-ui: the lock screen is ready, shown ${lock.shown}`);

const overview = Overview.get();
gnoblin.input.primary.bind("Super+o", () => {
    console.log(`shell-ui: the overview is ${overview.toggle() ? "on" : "off"}`);
});

gnoblin.input.primary.bind("Super+space", () => {
    Launcher.open((command) => console.log(`shell-ui: the launcher ran ${command}`));
    console.log("shell-ui: the launcher is open");
});

console.log("shell-ui: running");
