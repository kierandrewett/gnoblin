// The stock titlebar: a `Box` row with a drag area that holds the title and the minimise, maximise and close buttons.
// It is the easy path onto a window frame and is built from the same parts a script puts in any outlet: `TitleText`,
// `WindowButton` and the drag behaviour of `DragArea`. `titlebarFill` puts one in the top outlet of a frame, which is
// what `Decorations` does unless a script fills the frame itself. design/window-frame.md, decision 11.
// Every look setting is a field of `TitlebarStyle`.
// The bar is a child of the window's node: above it when mounted alone, in the frame's `top` outlet otherwise.
// docs/js-ui.md, "Titlebar", and docs/js-window-frame.md say why.

import { WindowActions } from "gnoblin/frame/actions";
import type { FrameClient, FrameContext, FrameHooks } from "gnoblin/frame/client";
import type { FrameFill } from "gnoblin/frame/decorations";
import type { WindowFrame } from "gnoblin/frame/frame";
import { DragArea } from "gnoblin/frame/parts/drag_area";
import { TitleText } from "gnoblin/frame/parts/title_text";
import { defaultButtonIcons, WindowButton } from "gnoblin/frame/parts/window_button";
import type { WindowButtonName } from "gnoblin/frame/parts/window_button";
import { Box } from "gnoblin/ui";

/** The three window buttons, which a style can order, leave out or give other icons. */
export type TitlebarButton = WindowButtonName;

/** The look of a titlebar. Lengths are pixels. */
export interface TitlebarStyle {
    height: number;
    /** Space between the bar's edge and what it holds. */
    padding: number;
    /** Space between the drag area and the buttons, and between the buttons. */
    gap: number;
    radius: number;
    /** The fill while the window has the focus. */
    focusedFill: string;
    unfocusedFill: string;
    titleSize: number;
    titleColor: string;
    titleFont: string;
    /** The buttons, left to right. */
    buttons: readonly TitlebarButton[];
    buttonSize: number;
    buttonColor: string;
    /** The icon of each button, by name from the icon theme. */
    icons: Readonly<Record<TitlebarButton, string>>;
}

/** The look of the retired Lua shell's titlebar, with the window buttons beside the close button. */
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
    readonly client: FrameClient;
    style: TitlebarStyle;
    readonly hooks: FrameHooks;
    /** The part of the bar that holds the title: the bar's room that no button takes. */
    readonly dragArea: Box;
    readonly title: TitleText;
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

    /** The clock the double click and the fade are timed by, in milliseconds. */
    get now(): () => number {
        return this.context.actions.now;
    }

    set now(clock: () => number) {
        this.context.actions.now = clock;
    }

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
 * The frame callback of the stock decoration: a `Titlebar` in the top outlet of a server frame, and nothing for a window
 * that draws its own frame (it returns `false`, so that window has no frame at all). The edge is as thick as the style's
 * `height` is, now.
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
