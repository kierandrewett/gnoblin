// KioskLayout shows one node at a time in the full area. It shows the focused child, or the first child if focus is
// elsewhere; hidden nodes remain children and are tiled again when shown.

import { MonocleLayout } from "gnoblin-std/layouts/monocle";

/** One node at a time, full area. It is `MonocleLayout` under the name a configuration gives the intent. */
export class KioskLayout extends MonocleLayout {}
