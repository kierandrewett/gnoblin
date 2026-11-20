import type { FrameContext } from "gnoblin-std/frame/client";
import { Box } from "gnoblin-std/ui/box";
import type { BoxOptions } from "gnoblin-std/ui/box";
import type { PaintOp } from "gnoblin-std/ui/paint";
/** Options of an `AppIcon`: a `Box`'s, and the tile's lettering. */
export interface AppIconOptions extends BoxOptions {
    /** The side of the tile. */
    size?: number;
    /** The lettering colour. */
    color?: string;
}
/** The letter and colour of an application id: the last part of `org.gnome.Nautilus` is `Nautilus`, so `N`. */
export declare function appTile(appId: string): {
    letter: string;
    colour: string;
};
/** A rounded square with the application's initial, `size` pixels a side. */
export declare class AppIcon extends Box {
    #private;
    /** The window whose application id determines the tile. */
    readonly context: FrameContext;
    /** The square's side in logical pixels. */
    size: number;
    /** Colour used for the initial letters. */
    color: string;
    constructor(context: FrameContext, options?: AppIconOptions);
    protected naturalWidth(): number;
    protected naturalHeight(): number;
    /** Updates the letter and tile colour from the window's current application id. */
    refresh(): void;
    /** Draws the tile and its application initial. */
    draw(): PaintOp[];
}
