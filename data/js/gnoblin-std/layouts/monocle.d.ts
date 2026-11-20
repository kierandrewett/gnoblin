import { NativeLayout } from "gnoblin-std/layouts/native";
import type { OptionSpecs } from "gnoblin-std/layouts/native";
/**
 * Shows one node full size and hides the rest. A hidden node keeps its place in the tree, so it is tiled again when
 * shown.
 */
export declare class MonocleLayout extends NativeLayout {
    protected get kind(): number;
    protected get options(): OptionSpecs;
}
