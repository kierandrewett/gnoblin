import type { WindowActions } from "gnoblin-std/frame/actions";
import type { Rect } from "gnoblin-core/types";
/** What a frame and its parts read and write of the window they frame. A `Client` of the host module is one. */
export interface FrameClient {
    /** The title shown by the window. */
    readonly title: string;
    /** Whether the window currently has keyboard focus. */
    readonly focused: boolean;
    /** Whether the window currently fills the output. */
    readonly fullscreen: boolean;
    /** The content's size: the frame is the content with its inset around it. */
    readonly width: number;
    /** The content height in logical pixels, when known. */
    readonly height?: number;
    /** The window's application id, which `AppIcon` and a rule read. A client that does not say has none. */
    readonly appId?: string;
    /** False once the compositor has destroyed the window. A client that does not say is alive. */
    readonly alive?: boolean;
    /** The content's x coordinate in logical pixels. */
    x: number;
    /** The content's y coordinate in logical pixels. */
    y: number;
    /** Whether the window is maximised. */
    maximised: boolean;
    /** Whether the window is minimised. */
    minimised: boolean;
    /** The content's rectangle, which a `ResizeHandle` writes. A client that has none cannot be resized by a handle. */
    bounds?: Rect;
    /** The least a handle resizes the window to. Absent, the stock minimum. */
    minSize?: {
        width: number;
        height: number;
    };
    /** The most a handle resizes the window to. Absent means no limit. */
    maxSize?: {
        width: number;
        height: number;
    };
    /** Asks the client to close the window. */
    close(): void;
    /**
     * Moves `opacity` and `scale` over time: the fade of a closing window. Without it the window just closes.
     */
    set?(values: {
        opacity?: number;
        scale?: [number, number, number];
    }, options: {
        duration: number;
        easing?: "ease-in" | "ease-out";
    }): unknown;
}
/** What the seat's click actions do that a frame cannot do alone. The host supplies them. */
export interface FrameHooks {
    /** The seat's `menu` action: open the window menu where the pointer is. */
    menu?(client: FrameClient): void;
    /** The seat's `lower` action: put the window under the others. */
    lower?(client: FrameClient): void;
    /**
     * Maximises or restores the window as the shell does it, which leaves room for the frame. Else the flag is
     * written.
     */
    maximise?(client: FrameClient, on: boolean): void;
    /**
     * Minimises the window as the shell does it: the keyboard focus goes to another window. Else the flag is written.
     */
    minimise?(client: FrameClient): void;
    /** The milliseconds an animation takes: 0 while the seat asks for reduced motion. Absent, the time it was given. */
    motion?(duration: number): number;
    /**
     * A button went down on the frame itself, not on one of its buttons: the window is pressed and takes the focus.
     */
    press?(client: FrameClient, event: {
        x: number;
        y: number;
        button: number;
    }): void;
    /** The button that went down on the frame came up. */
    release?(client: FrameClient, event: {
        x: number;
        y: number;
        button: number;
    }): void;
}
/** What a part is made for: the window, the shell's hooks and the actions every part of the window shares. */
export interface FrameContext {
    /** The window being framed. */
    readonly client: FrameClient;
    /** Shell operations used by frame parts. */
    readonly hooks: FrameHooks;
    /** Shared actions available to frame parts. */
    readonly actions: WindowActions;
}
