// Example 31 in TypeScript: the parts of the stock shell above the toolkit, from libgnoblin-std.
//
//     gnoblin ./shell.ts
//
// The classes are generated from the catalog the library serves, so `gnoblin-std` has one for each of them. This shell:
//
// - starts the not-responding alert, which looks at the windows and opens a card for one that stops answering the compositor's pings: Escape
//   waits, Tab moves to Force quit, and Force quit takes a press only after `armMs`;
// - puts the stock lock screen to work: a clock and a password entry over the screen while the session is locked;
// - opens the launcher with `Super+space`: Return runs the command typed and closes it, Escape closes it;
// - flips the overview with `Super+o`;
// - shows a toast with `Super+t`;
// - calls the toolkit's two functions that need no canvas: `Ui.estimateText` and `Ui.encodePaintList`.
//
// Every line it logs starts with `shell-ui:`, spelt as the Rust shell of the same example (`31-native-shell-ui`) spells it, so that one test
// reads all three.
import { gnoblin } from "gnoblin-core";
import { Launcher, LockScreen, NotRespondingAlerts, Overview, Toast, Ui } from "gnoblin-std";
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

// A toast fades in for 20 ms, holds for 400 ms and fades out for 20 ms, then removes itself.
gnoblin.input.primary.bind("Super+t", () => {
    Toast.show("Toast of the shell", 20, 400, 20);
    console.log("shell-ui: the toast is up");
});

// What a text is taken to measure until it is measured: 5 pixels for each of 5 characters at size 10, a line of 15, a baseline at 11.
const estimate = Ui.estimateText("Hello", 10);
console.log(`shell-ui: estimate ${estimate.width}x${estimate.height} ascent ${estimate.ascent}`);

// The bytes of a paint list: the magic `GNPL`, a version, a count, then the operations. A list with one bad operation is refused whole.
// The line's width is its `width`, as it is in the TypeScript library: the bytes carry it as a little-endian f32 after the line's four ends.
const list = Ui.encodePaintList([
    { op: "push-opacity", alpha: 0.5 },
    { op: "line", x1: 0, y1: 0, x2: 5, y2: 6, width: 3 },
    { op: "pop-opacity" },
]);
const lineWidth = new DataView(list.buffer, list.byteOffset + 31, 4).getFloat32(0, true);
console.log(
    `shell-ui: paint list of ${list.length} bytes ${String.fromCharCode(...list.slice(0, 4))}, line width ${lineWidth}`,
);
try {
    Ui.encodePaintList([{ op: "pop-opacity" }, { op: "rect", fill: "red" }]);
} catch (error) {
    console.log(`shell-ui: bad paint list refused: ${(error as Error).message}`);
}

console.log("shell-ui: running");
