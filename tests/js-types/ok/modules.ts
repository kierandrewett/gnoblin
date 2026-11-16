// Every module of the library a script names resolves, under the name the runtime loads it by.
import { InputContext } from "gnoblin-core/input";
import { Style } from "gnoblin-std/style";
import { Box, Canvas } from "gnoblin-std/ui";

export { Box, Canvas, InputContext, Style };
// The global `gnoblin` is the library's `Gnoblin`, so a member of it type-checks against the class that implements it.
export const clientCount: number = gnoblin.clients.length;
