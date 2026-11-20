import type { Client } from "gnoblin-core/host/client";
import { MutableCollection } from "gnoblin-core/host/collections";
import type { Output } from "gnoblin-core/host/output";
import { NativeObject } from "gnoblin-core/native_object";
import type { PropertyBinding, PropertyBindings } from "gnoblin-core/properties";
import { Workspace as NativeWorkspace } from "gnoblin-std/generated/classes";
import type { ShowOptions } from "gnoblin-std/host/show";
import type { Gaps, Layout } from "gnoblin-std/layout";
import type { Transition } from "gnoblin-std/transition/transition";
import type { TransitionOptions } from "gnoblin-std/transition/types";
import type { StockLayoutName, WorkspaceOptions } from "gnoblin-std/workspace";
/**
 * A place windows are laid out in. The script makes it and adds it to `gnoblin.workspaces`; `clients` holds
 * its windows, the same `Client` objects as `gnoblin.clients`. Removing a workspace, or a window from one,
 * never closes the window.
 */
export declare class Workspace extends NativeObject {
    #private;
    constructor(options: WorkspaceOptions);
    /** Wraps a workspace the library made itself (the default `desktop`, a per-output policy's): the one object for its handle. */
    static wrap(native: NativeWorkspace): Workspace;
    /** Stable workspace id used to find it across reloads. */
    get id(): string;
    /** Display name shown by workspace controls. */
    get name(): string;
    /** Sets the workspace display name. */
    set name(value: string);
    /** A workspace is the script's own and stays usable after it leaves `gnoblin.workspaces`. */
    get alive(): boolean;
    /** The position in `gnoblin.workspaces`, or -1 while it is not in it. */
    get index(): number;
    /** Whether an output shows this workspace. With several outputs, several workspaces are active. */
    get active(): boolean;
    /** The output that shows this workspace, or null while none does. */
    get output(): Output | null;
    /** The layout instance used to place this workspace's windows. */
    get layout(): Layout;
    /** A layout instance, or the name of a stock layout. The windows stay where they are until it computes. */
    set layout(value: Layout | StockLayoutName);
    /** Inner and outer gaps, in logical pixels. */
    get gaps(): Gaps;
    /** Sets inner and outer gaps, in logical pixels. */
    set gaps(value: {
        inner?: number;
        outer?: number;
    });
    /** Mutable collection of this workspace's windows. */
    get clients(): MutableCollection<Client>;
    /**
     * Assigns `props` (`layout`, `gaps`, `name`) as one `requestTransition`: the windows tween from where they are
     * drawn to where the new layout puts them. Without `options` the change is atomic and immediate.
     */
    set(props: Record<string, unknown>, options?: TransitionOptions): Transition;
    /** The reactive properties of the workspace: `workspace.properties.bind("blurRadius", () => ...)`. */
    get properties(): PropertyBindings;
    /** Shorthand for `workspace.properties.bind`. */
    bind(property: string, compute: () => unknown): PropertyBinding;
    /** Shows this workspace on the output under the pointer (the primary output when no pointer says). */
    show(options?: ShowOptions): void;
    /**
     * Moves the keyboard focus `step` windows along the workspace's windows, in the order they are listed: `1` to
     * the next, `-1` to the previous. Nothing moves when no window of the workspace has the focus, and a step past
     * either end leaves the focus where it is.
     */
    walk(step: number): void;
    /** Returns a label containing the workspace id. */
    toString(): string;
}
