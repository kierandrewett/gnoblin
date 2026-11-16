// The stock titlebar, built from the same `Box`, `TitleText`, `WindowButton` and `DragArea` parts available to scripts.
// `titlebarFill` adds it to a server frame's top outlet; `Decorations` uses it unless a script fills the frame itself.
// The bar is a child of the window's node: above it when mounted alone, in the frame's `top` outlet otherwise.
// See docs/js-ui.md, "Titlebar", and docs/js-window-frame.md for usage.

import { WindowActions } from "gnoblin-std/frame/actions";
import type { FrameClient, FrameContext, FrameHooks } from "gnoblin-std/frame/client";
import type { FrameFill } from "gnoblin-std/frame/decorations";
import type { WindowFrame } from "gnoblin-std/frame/frame";
import { DragArea } from "gnoblin-std/frame/parts/drag_area";
import { TitleText } from "gnoblin-std/frame/parts/title_text";
import { defaultButtonIcons, WindowButton } from "gnoblin-std/frame/parts/window_button";
import type { WindowButtonName } from "gnoblin-std/frame/parts/window_button";
import { Box } from "gnoblin-std/ui/box";

/** The three window buttons, which a style can order, leave out or give other icons. */
export type TitlebarButton = WindowButtonName;

/** The titlebar's appearance and layout. Lengths are logical pixels. */
export interface TitlebarStyle {
    /** Bar height in logical pixels. */
    height: number;
    /** Space between the bar's edge and what it holds. */
    padding: number;
    /** Space between the drag area and the buttons, and between the buttons. */
    gap: number;
    /** Corner radius in logical pixels. */
    radius: number;
    /** The fill while the window has the focus. */
    focusedFill: string;
    /** Fill when the window is not focused. */
    unfocusedFill: string;
    /** Title text size in logical pixels. */
    titleSize: number;
    /** Title text colour. */
    titleColor: string;
    /** Title font family. */
    titleFont: string;
    /** The buttons, left to right. */
    buttons: readonly TitlebarButton[];
    /** Button icon size in logical pixels. */
    buttonSize: number;
    /** Button icon colour. */
    buttonColor: string;
    /** The icon of each button, by name from the icon theme. */
    icons: Readonly<Record<TitlebarButton, string>>;
}

/** The default titlebar appearance and button order. */
export const defaultTitlebarStyle: Readonly<TitlebarStyle> = {
    height: 28,
    padding: 6,
    gap: 8,
    radius: 0,
    focusedFill: "#414559",
    unfocusedFill: "#232634",
    titleSize: 13,
    titleColor: "#c6d0f5",
    titleFont: "sans",
    buttons: ["minimise", "maximise", "close"],
    buttonSize: 16,
    buttonColor: "#cdd6f4",
    icons: defaultButtonIcons,
};

/**
 * The bar. Its width follows the client's, its fill follows the focus and its title the client's title, all read
 * once a frame in `refresh`. A press on a button does what the button says; a press elsewhere on the bar drags the
 * window, and a second press in quick succession, a middle click and a right click do what the seat's `double_click`,
 * `middle_click` and `right_click` say (`toggle-maximize`, `lower`, `menu`, `minimize` or `none`).
 *
 * The close button fades the window out first (`CLOSE_FADE_MS`) and then closes it; see `WindowActions`.
 *
 * Mounted on a window's node it lies above the window, its own size; put in a frame's `top` outlet it takes the
 * outlet's room, and the frame's actions are its own (`actions`).
 */
export class Titlebar extends DragArea {
    /** Window whose title and state the bar displays. */
    readonly client: FrameClient;
    /** Mutable appearance applied during `refresh()`. */
    style: TitlebarStyle;
    /** Hooks used by the frame actions. */
    readonly hooks: FrameHooks;
    /** The part of the bar that holds the title: the bar's room that no button takes. */
    readonly dragArea: Box;
    /** Title text shown by the bar. */
    readonly title: TitleText;
    /** Buttons currently present in the bar, keyed by action name. */
    readonly buttons = new Map<TitlebarButton, WindowButton>();
    /** Whether the bar runs the close fade itself: it does unless it was given a frame's shared actions. */
    readonly #ownsActions: boolean;

    constructor(
        client: FrameClient,
        style: TitlebarStyle = { ...defaultTitlebarStyle },
        hooks: FrameHooks = {},
        actions?: WindowActions,
    ) {
        const shared = actions ?? new WindowActions(client, hooks);
        const context: FrameContext = { client, hooks, actions: shared };
        super(context, {
            direction: "row",
            height: style.height,
            padding: style.padding,
            gap: style.gap,
            radius: style.radius,
        });
        this.client = client;
        this.style = style;
        this.hooks = hooks;
        this.#ownsActions = actions === undefined;
        this.title = new TitleText(context, {
            size: style.titleSize,
            color: style.titleColor,
            font: style.titleFont,
        });
        this.dragArea = new Box({ direction: "row", grow: 1, children: [this.title] });
        this.add(this.dragArea);
        for (const name of style.buttons) {
            const button = new WindowButton(context, name, {
                icon: style.icons[name],
                size: style.buttonSize,
                color: style.buttonColor,
            });
            this.buttons.set(name, button);
            this.add(button);
        }
    }

    /** Clock used to time double clicks and close fades, in milliseconds. */
    get now(): () => number {
        return this.context.actions.now;
    }

    /** Sets the clock used to time double clicks and close fades. */
    set now(clock: () => number) {
        this.context.actions.now = clock;
    }

    /** Updates the bar's size, style, title, focus fill and visibility from its window. */
    override refresh(): void {
        const { client, style } = this;
        this.height = style.height;
        if (this.parent === null) {
            // Mounted on the window's node: above the content, whose origin is the node's corner, and as wide as it.
            this.width = client.width;
            this.x = 0;
            this.y = -style.height;
        } else {
            // In an outlet: the outlet's row places the bar and gives it the room.
            this.grow = 1;
        }
        this.padding = style.padding;
        this.gap = style.gap;
        this.radius = style.radius;
        this.fill = client.focused ? style.focusedFill : style.unfocusedFill;
        this.visible = !client.fullscreen;
        this.title.size = style.titleSize;
        this.title.color = style.titleColor;
        for (const [name, button] of this.buttons) {
            button.icon = style.icons[name];
            button.size = style.buttonSize;
            button.color = style.buttonColor;
        }
        if (this.#ownsActions) {
            this.context.actions.advance();
        }
    }
}

/**
 * Adds a `Titlebar` to the top outlet of a server frame. Returns `false` for client-drawn frames, which have no server
 * frame. The outlet height follows the style.
 */
export function titlebarFill(style: TitlebarStyle): FrameFill {
    return (_client: FrameClient, frame: WindowFrame): false | void => {
        if (!frame.server) {
            return false;
        }
        const bar = new Titlebar(frame.client, style, frame.hooks, frame.actions);
        frame.top.size = style.height;
        frame.top.add(bar);
        frame.top.onRefresh = () => {
            frame.top.size = bar.style.height;
        };
    };
}
