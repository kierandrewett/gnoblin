// What a window frame needs of a window and of the shell around it: `FrameClient`, `FrameHooks` and `FrameContext`.
// The frame never names the host's `Client`; a `Client` satisfies `FrameClient`, and a plain object does for a test.
// design/window-frame.md, decisions 9 and 10.

import type { WindowActions } from "gnoblin/frame/actions";
import type { Rect } from "gnoblin/types";

/** What a frame and its parts read and write of the window they frame. A `Client` of the host module is one. */
export interface FrameClient {
    readonly title: string;
    readonly focused: boolean;
    readonly fullscreen: boolean;
    /** The content's size: the frame is the content with its inset around it. */
    readonly width: number;
    readonly height?: number;
    /** The window's application id, which `AppIcon` and a rule read. A client that does not say has none. */
    readonly appId?: string;
    /** False once the compositor has destroyed the window. A client that does not say is alive. */
    readonly alive?: boolean;
    x: number;
    y: number;
    maximised: boolean;
    minimised: boolean;
    /** The content's rectangle, which a `ResizeHandle` writes. A client that has none cannot be resized by a handle. */
    bounds?: Rect;
    /** The least a handle resizes the window to. Absent, the stock minimum. */
    minSize?: { width: number; height: number };
    /** The most a handle resizes the window to. Absent means no limit. */
    maxSize?: { width: number; height: number };
    close(): void;
    /**
     * Moves `opacity` and `scale` over time: the fade of a closing window. Without it the window just closes.
     */
    animate?(
        values: { opacity?: number; scale?: [number, number, number] },
        options: { duration: number; easing?: "ease-in" | "ease-out" },
    ): void;
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
    press?(client: FrameClient, event: { x: number; y: number; button: number }): void;
    /** The button that went down on the frame came up. */
    release?(client: FrameClient, event: { x: number; y: number; button: number }): void;
}

/** What a part is made for: the window, the shell's hooks and the actions every part of the window shares. */
export interface FrameContext {
    readonly client: FrameClient;
    readonly hooks: FrameHooks;
    readonly actions: WindowActions;
}
