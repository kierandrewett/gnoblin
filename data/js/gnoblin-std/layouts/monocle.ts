// MonocleLayout shows the focused node at full size and hides the others without moving them. If focus is elsewhere,
// it shows the first child. Hidden nodes remain in the tree and are tiled again when shown. The algorithm is the library's
// (`libgnoblin-std`, kind `monocle`). See `gnoblin-std/layout` for the `Layout` base class.

import { BuiltinKind } from "gnoblin-std/generated/constants";
import { NativeLayout } from "gnoblin-std/layouts/native";
import type { OptionSpecs } from "gnoblin-std/layouts/native";

/**
 * Shows one node full size and hides the rest. A hidden node keeps its place in the tree, so it is tiled again when
 * shown.
 */
export class MonocleLayout extends NativeLayout {
    protected get kind(): number {
        return BuiltinKind.MONOCLE;
    }

    protected get options(): OptionSpecs {
        return {};
    }
}
