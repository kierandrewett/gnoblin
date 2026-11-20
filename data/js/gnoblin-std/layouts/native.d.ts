import { Builtin } from "gnoblin-std/generated/classes";
import type { Option, Placement as LibraryPlacement } from "gnoblin-std/generated/records";
import type { Init } from "gnoblin-std/generated/native";
import type { Point } from "gnoblin-core/bindings/types";
import type { LayoutSplit } from "gnoblin-std/bindings/tiling";
import { Layout } from "gnoblin-std/layout";
import type { LayoutContext, LayoutNode, Placement } from "gnoblin-std/layout";
/** What the library's actions take of a workspace: the nodes it holds, and which of them has the keyboard. */
export interface ActionSource {
    /** The nodes the layout places, in the workspace's order. A node with a `focused` flag that is true has the keyboard. */
    readonly clients: Iterable<LayoutNode>;
}
/** The size of a node with its frame, for an action that keeps a window's own size. */
export interface OwnSize {
    /** Width in logical pixels. */
    width: number;
    /** Height in logical pixels. */
    height: number;
}
/** What a script's option is called in the library, and the words of its enumeration when it has one. */
export interface OptionSpec {
    /** The option's name in the library's table of layout kinds (`master_count`). */
    readonly native: string;
    /** The members of an enumeration option, by the word a script writes (`left`) in upper case (`LEFT`). */
    readonly members?: Readonly<Record<string, number>>;
    /** The library's value for the script's (`"auto"` columns are 0). */
    readonly toNative?: (value: unknown) => unknown;
    /** The script's value for the library's. */
    readonly fromNative?: (value: unknown) => unknown;
    /** True for an option only the script changes (a function the library is told the meaning of): it is not read back. */
    readonly readOnly?: boolean;
}
/** The options of a layout, by the name of the script's own field. */
export type OptionSpecs = Readonly<Record<string, OptionSpec>>;
/** The number of `node`, given on first use. */
export declare function numberOf(node: object): number;
/** One argument of an action as the library takes it: absent when `value` is `undefined` or `null`, a member of `members` by its word. */
export declare function argOf(name: string, value: unknown, members?: Readonly<Record<string, number>>): Init<Option>;
/** A node as an argument of an action: the number the library knows it by. */
export declare function nodeArgOf(name: string, node: LayoutNode): Init<Option>;
/**
 * A stock layout the library computes. A subclass names its kind and its options and keeps them as own fields; everything else is
 * the library's.
 */
export declare abstract class NativeLayout extends Layout {
    #private;
    /** The `STD_BUILTIN_*` kind of the library's layout. */
    protected abstract get kind(): number;
    /** The options, by the name of the field that holds each. */
    protected abstract get options(): OptionSpecs;
    /** The client of `context.children` that the library is to treat as the focused one. */
    protected focusedChild(context: LayoutContext): LayoutNode | undefined;
    /** The seat's pointer in the layout's coordinates, for a layout that reads it. */
    protected pointerOf(): Point | null;
    /** The own size of `node` with its frame, for an action that keeps a window's size; null when the node has none. */
    protected ownSizeOf(_node: LayoutNode): OwnSize | null;
    /** Called when the library changed something a workspace cannot see in the fields: a divider moved, an action changed the tree. */
    protected changed(): void;
    /** What the library is asked about: the clients, the area, the gaps, the focus and what the layout reads of the pointer. */
    protected query(context: LayoutContext, focused: LayoutNode | undefined): {
        flags: number;
        children: number[];
        area: import("../../gnoblin-core").Rect;
        gapInner: number;
        gapOuter: number;
        focused: number;
        pointerX: number;
        pointerY: number;
    };
    /** The library's layout with the options the fields hold now. */
    protected native(): Builtin;
    /** Reads the options back into the fields: the library moved one (a divider dragged, an action that sets a ratio). */
    protected pull(): void;
    /** Where `context.children` go, as the library computes it. */
    compute(context: LayoutContext): Placement[];
    /** A placement of the library as the script's `Placement`. */
    protected placement(raw: LibraryPlacement): Placement;
    /**
     * Runs the library's action `name` with `values` against the nodes of `source` and the one that has the keyboard. A node the
     * library hands the keyboard to is given it. The options are read back, and an answer that is not `false` says the action
     * changed something (`changes` false for one that only reads or sets what no workspace compares).
     */
    protected act(name: string, values: readonly Init<Option>[], source: ActionSource, changes?: boolean): unknown;
    /** The divider a drag that pressed `client` at `point` would move: the library's, or null while it has none. */
    findSplit(client: LayoutNode, point: Point): LayoutSplit | null;
    /** Moves the dividers of the drag `token`, and reads the options back. */
    setSplit(token: number, ratio: number, cross: number): void;
    /** Puts the dividers of the drag `token` back, and reads the options back. */
    resetSplit(token: number): void;
    /** The two clients traded places in the workspace's order. */
    onClientsSwapped(a: LayoutNode, b: LayoutNode): void;
    /**
     * A client left the workspace: a layout that keeps the keyboard in its own order (a strip's columns) hands it on. Only such a layout
     * declares `onClientRemoved` (the host leaves the hand-over to a layout that has the hook), and it calls this.
     */
    protected handOnFocus(client: LayoutNode, context: LayoutContext): void;
}
