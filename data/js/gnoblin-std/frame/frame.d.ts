import { WindowActions } from "gnoblin-std/frame/actions";
import type { FrameClient, FrameContext, FrameHooks } from "gnoblin-std/frame/client";
import { CornerOutlet } from "gnoblin-std/frame/corner_outlet";
import { EdgeOutlet } from "gnoblin-std/frame/edge_outlet";
import type { FrameInset } from "gnoblin-core/inset";
import { LayerOutlet } from "gnoblin-std/frame/layer_outlet";
import type { FrameGeometry, Outlet, OutletOwner } from "gnoblin-std/frame/outlet";
import { ResizeHandles } from "gnoblin-std/frame/parts/resize_handles";
import type { ResizeHandlesOptions } from "gnoblin-std/frame/parts/resize_handles";
import type { Rect } from "gnoblin-core/types";
/**
 * What a frame is for: `server` for a window the shell frames (it has edges and an inset), `layers` for a window that
 * draws its own frame or has none, which can still have an underlay and an overlay but no edge and no inset.
 */
export type FrameKind = "server" | "layers";
/** The content outlet: where the client's surface sits. It is not filled, and says where it is. */
export interface ContentOutlet {
    /** The content's rectangle in the window's coordinates: its origin is the window node's. */
    readonly rect: Rect;
    /** The room the frame leaves round it. */
    readonly inset: FrameInset;
}
/**
 * The frame of one window. Its outlets are `top`, `bottom`, `left` and `right` (edges that take room), `topLeft`,
 * `topRight`, `bottomLeft` and `bottomRight` (corners that take none), `underlay` and `overlay` (layers that take
 * none), `start`, `center` and `end` (the top edge's zones), and `content` (not fillable). Reading an outlet of a
 * client-drawn frame (`kind` `layers`) throws, except for the layers: a second title bar over the client's own is a
 * double title bar.
 *
 * `inset` is the space left around content, and is all zero while the window is fullscreen; `reserved` is the same
 * without that, the room the window keeps while fullscreen hides the frame.
 */
export declare class WindowFrame implements FrameContext, OutletOwner {
    #private;
    /** The window this frame surrounds. */
    readonly client: FrameClient;
    /** Shell operations used for window actions. */
    readonly hooks: FrameHooks;
    /** Shared actions that frame parts use to operate on the window. */
    readonly actions: WindowActions;
    /** Whether the frame uses shell-drawn edges and reserves an inset. */
    readonly kind: FrameKind;
    /**
     * `node` is the window's node in the tree: the outlets are made under it. `actions` is the window's shared actions;
     * left out, the frame makes them from `hooks`.
     */
    constructor(client: FrameClient, node: number, options?: {
        kind?: FrameKind;
        hooks?: FrameHooks;
        actions?: WindowActions;
    });
    /** Whether the frame is a server frame: the window has edges and an inset. */
    get server(): boolean;
    /** True until `destroy`, and until the window's node is gone. */
    get alive(): boolean;
    /** The content's size and the inset now. */
    geometry(): FrameGeometry;
    /** Room reserved around the content, or zero when fullscreen or without a server frame. */
    get inset(): FrameInset;
    /** The room the frame keeps for itself even while fullscreen hides it. */
    get reserved(): FrameInset;
    /** The content outlet: not fillable, it says where the client's surface is and how much room the edges left it. */
    get content(): ContentOutlet;
    /** The top edge outlet. */
    get top(): EdgeOutlet;
    /** The bottom edge outlet. */
    get bottom(): EdgeOutlet;
    /** The left edge outlet. */
    get left(): EdgeOutlet;
    /** The right edge outlet. */
    get right(): EdgeOutlet;
    /** The top-left corner outlet. */
    get topLeft(): CornerOutlet;
    /** The top-right corner outlet. */
    get topRight(): CornerOutlet;
    /** The bottom-left corner outlet. */
    get bottomLeft(): CornerOutlet;
    /** The bottom-right corner outlet. */
    get bottomRight(): CornerOutlet;
    /** The first zone of the top edge: a titlebar's left. */
    get start(): Outlet;
    /** The middle zone of the top edge: a titlebar's title, which takes the room the other two leave. */
    get center(): Outlet;
    /** The last zone of the top edge: a titlebar's right. */
    get end(): Outlet;
    /** The layer behind the window's content: a backdrop, a glow. It takes no room and no pointer. */
    get underlay(): LayerOutlet;
    /**
     * Layer in front of the window for a focus ring, badge or resize handles. It takes no room and, by default, no
     * pointer.
     */
    get overlay(): LayerOutlet;
    /**
     * Puts the eight resize handles round the frame, with `thickness` pixels reaching outside it, and returns them.
     * The stock titlebar has none: resize with `Super+MouseRight+Drag` unless the frame has handles.
     */
    resizeHandles(thickness?: number, options?: Omit<ResizeHandlesOptions, "thickness">): ResizeHandles;
    /** Takes the frame away: its outlets and everything in them are destroyed, and its hook stops. */
    destroy(): void;
}
