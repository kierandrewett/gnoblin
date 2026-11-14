// KioskLayout: one node at a time, full area. The shown node is the focused one when it is a child, otherwise the
// first child; every other node is hidden and stays a child, so it is still tiled when shown again. Geometry is the
// Lua shell's `Kiosk` in data/lua/gnoblin/shell/tiling.lua, which places exactly as `Monocle` does: the two differ
// only in the name a configuration gives the intent.

import { MonocleLayout } from "gnoblin/layouts/monocle";

/** One node at a time, full area. It is `MonocleLayout` under the name a configuration gives the intent. */
export class KioskLayout extends MonocleLayout {}
