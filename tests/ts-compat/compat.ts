// Declaration compatibility of the native std port (design/native-std-port.md, 3.6 item 8, lane NpOracle).
//
// `old-surface/` is the declaration emit of the TypeScript library as it stood before the port (`tsc --declaration --emitDeclarationOnly`
// over `data/js/gnoblin-core` and `data/js/gnoblin-std`, TypeScript 7.0.2, strict). Each class below that the new generated
// `gnoblin-std.d.ts` declares must be assignable to the old one: an instance of the new class can be used wherever the old one was.
//
// `gnoblin-std-next` names the declarations under test. While NpGenOut has not generated any, `tsconfig.json` maps it to the old tree, so the
// file type-checks against the old declarations with the new file absent. The lane that generates `gnoblin-std.d.ts` changes that one
// `paths` entry. NpGate deletes this directory once the owner signs off the new surface.

import * as Old from "gnoblin-std";
import * as Next from "gnoblin-std-next";

type Same<K extends keyof typeof Old & keyof typeof Next> = (typeof Next)[K] extends abstract new (
    ...args: never
) => infer N
    ? (typeof Old)[K] extends abstract new (...args: never) => infer O
        ? N extends O
            ? true
            : never
        : never
    : never;

// One line for each class a shell extends or constructs. A class that fails shows as `never` in the tuple and the assignment errors.
const classes: [
    Same<"FloatingLayout">,
    Same<"GridLayout">,
    Same<"MasterStackLayout">,
    Same<"MonocleLayout">,
    Same<"DwindleLayout">,
    Same<"KioskLayout">,
    Same<"ArcLayout">,
    Same<"StripLayout">,
    Same<"Layout">,
    Same<"Canvas">,
    Same<"Box">,
    Same<"Grid">,
    Same<"Text">,
    Same<"Icon">,
    Same<"Entry">,
    Same<"Clock">,
    Same<"Bar">,
    Same<"Titlebar">,
    Same<"Decorations">,
    Same<"WindowFrame">,
    Same<"WindowActions">,
    Same<"WindowMenus">,
    Same<"Node">,
    Same<"View">,
    Same<"Scene">,
    Same<"Camera3D">,
    Same<"Camera2D">,
    Same<"Transition">,
    Same<"LockScreen">,
    Same<"PasswordEntry">,
    Same<"ClientMoveBinding">,
    Same<"ClientResizeBinding">,
    Same<"ClientSwapBinding">,
    Same<"SplitResizeBinding">,
] = new Array(34).fill(true) as never;

export { classes };
