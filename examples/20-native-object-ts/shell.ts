// Example 20 in TypeScript: the object classes of libgnoblin-std.
//
//     gnoblin ./shell.ts
//
// The classes of the `gnoblin.std` catalog (an entry model, the focus policy, the records and enums they take) are not written by hand: they
// are generated from the catalog the library serves and call it through one object table, so `gnoblin-std/generated/objects` has a class
// for each of them and the editor's declarations come from the same catalog. This shell uses them the way a toolkit would:
//
// - makes an `EntryModel` and reads its properties (`text`, `length`, `caret`, `selection`);
// - calls its methods (`insert`, `selectAll`), which change it at once, and listens to its `changed` signal, which the library tells from its
//   next turn and never inside the call that caused it;
// - calls a static function of the focus policy with records, and reads the record it answers with (a field the library leaves out is
//   `undefined`);
// - keeps the entry in the closure of a chord: `Super+e` types one more `!` into it.
//
// Every line it logs starts with `object:`, spelt as the Rust shell of the same example (`20-native-object`) spells it, so that one test
// reads all three.
import { gnoblin } from "gnoblin-core";
import { EntryModel, FocusPolicy } from "gnoblin-std/generated/objects";
// The core's types are completed by `gnoblin-std` (it adds `workspace` to a client), so a shell that is checked against them names it.
// Nothing of it runs: the import is erased.
import type {} from "gnoblin-std";

// Made with its text; the caret starts at the end, and positions count characters, not bytes.
const entry = new EntryModel("héllo");
console.log(`object: made an entry of ${entry.length} characters with the caret at ${entry.caret}`);

// The library tells `changed` from its next turn, so these lines come after the ones above and below.
let heard = 0;
entry.on("changed", () => {
    heard += 1;
    console.log(`object: heard changed #${heard}`);
});

entry.insert(", world");
entry.selectAll();
const selection = entry.selection;
const as = selection === undefined ? "None" : `Some(TextRange { start: ${selection.start}, end: ${selection.end} })`;
console.log(`object: text ${JSON.stringify(entry.text)} selected ${JSON.stringify(entry.selectedText)} as ${as}`);

// Typing over a selection replaces it, so put the caret at the end first: `extend` is an optional argument and is left out.
entry.end();

// A function of the focus policy: records go in, a record comes out, and a field the caller leaves out has its default.
const action = FocusPolicy.focusDecision({ mode: "sloppy" }, { kind: "enter", held: false });
console.log(`object: a pointer entering a window under sloppy focus: focus ${action.focus}, raise ${action.raise}`);

gnoblin.input.primary.bind("Super+e", () => {
    const before = heard;
    entry.insert("!");
    console.log(`object: typed, text is ${JSON.stringify(entry.text)}, heard ${before} so far`);
});

console.log("object: running");
