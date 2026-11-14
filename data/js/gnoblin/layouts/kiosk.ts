// KioskLayout: one node at a time, full area. The shown node is the focused one when it is a child, otherwise the
// first child; every other node is hidden and stays a child, so it is still tiled when shown again. Geometry is
// data/lua/gnoblin/shell/tiling.lua `Kiosk`, which places exactly as `Monocle` does: the two differ only in the
// name a configuration gives the intent.

import { MonocleLayout } from "gnoblin/layouts/monocle";

export class KioskLayout extends MonocleLayout {}
