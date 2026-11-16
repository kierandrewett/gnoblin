// `AppIcon` is a tile for a window's application. The painter has no icon theme, so the tile shows the application's
// first letter on a colour derived from its id: the same application gets the same tile in every window and session.
// so the tile carries the first letter of its application id on a colour taken from the id: the same application is
// the same tile, in every window and every session.

import type { FrameContext } from "gnoblin-std/frame/client";
import { Box } from "gnoblin-std/ui/box";
import type { BoxOptions } from "gnoblin-std/ui/box";
import type { PaintOp } from "gnoblin-std/ui/paint";

/** The colours a tile is taken from, all dark enough for white lettering. */
const TILE_COLOURS = ["#5b6ee1", "#2f8f6b", "#b5533c", "#8a56ac", "#2d7d9a", "#a0762a", "#a8416a", "#4f7b38"] as const;

/** Options of an `AppIcon`: a `Box`'s, and the tile's lettering. */
export interface AppIconOptions extends BoxOptions {
    /** The side of the tile. */
    size?: number;
    /** The lettering colour. */
    color?: string;
}

/** The letter and colour of an application id: the last part of `org.gnome.Nautilus` is `Nautilus`, so `N`. */
export function appTile(appId: string): { letter: string; colour: string } {
    const parts = appId.split(".").filter((part) => part !== "");
    const name = parts[parts.length - 1] ?? "";
    let hash = 0;
    for (const char of appId) {
        hash = (hash * 31 + char.codePointAt(0)!) >>> 0;
    }
    return { letter: ([...name][0] ?? "?").toUpperCase(), colour: TILE_COLOURS[hash % TILE_COLOURS.length] };
}

/** A rounded square with the application's initial, `size` pixels a side. */
export class AppIcon extends Box {
    /** The window whose application id determines the tile. */
    readonly context: FrameContext;
    /** The square's side in logical pixels. */
    size: number;
    /** Colour used for the initial letters. */
    color: string;
    #appId = "";

    constructor(context: FrameContext, options: AppIconOptions = {}) {
        super({ radius: 4, ...options });
        this.context = context;
        this.size = options.size ?? 16;
        this.color = options.color ?? "#ffffff";
    }

    protected override naturalWidth(): number {
        return this.size;
    }

    protected override naturalHeight(): number {
        return this.size;
    }

    /** Updates the letter and tile colour from the window's current application id. */
    override refresh(): void {
        this.#appId = this.context.client.appId ?? "";
        this.fill = appTile(this.#appId).colour;
    }

    /** Draws the tile and its application initial. */
    override draw(): PaintOp[] {
        const side = Math.min(this.width, this.height);
        const size = Math.floor(side * 0.62);
        return [
            ...super.draw(),
            {
                op: "text",
                text: appTile(this.#appId).letter,
                x: Math.floor((this.width - Math.floor(size * 0.6)) / 2),
                y: Math.floor((this.height - Math.floor(size * 1.5)) / 2),
                size,
                color: this.color,
                font: "sans",
            },
        ];
    }
}
