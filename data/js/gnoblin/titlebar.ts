// The stock titlebar: a `Box` row of a drag area that holds the title, and the minimise, maximise and close
// buttons, drawn as a child of the window's node (design/node-api-refactor.md, decision 2a).
//
// Everything that sets how it looks is a plain field of `TitlebarStyle`, so a config can hand its own style over
// (`new Titlebar(client, { ...defaultTitlebarStyle, focusedFill: "#223" })`), and a later `defineConfig({ style })`
// can build one from its keys. `Decorations` gives every client one titlebar and takes it away when the client goes.

import { Box, Icon, Text } from "gnoblin/ui";
import type { UiPointerEvent } from "gnoblin/ui";

/** The three window buttons, which a style can order, leave out or give other icons. */
export type TitlebarButton = "minimise" | "maximise" | "close";

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

/** The look of the Lua shell's titlebar, with the window buttons beside the close button. */
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
    icons: {
        minimise: "window-minimize-symbolic",
        maximise: "window-maximize-symbolic",
        close: "window-close-symbolic",
    },
};

/** What a titlebar needs of the client it decorates. A `Client` of the host module is one. */
export interface TitlebarClient {
    readonly title: string;
    readonly focused: boolean;
    readonly fullscreen: boolean;
    readonly width: number;
    x: number;
    y: number;
    maximised: boolean;
    minimised: boolean;
    close(): void;
}

/** The left button of the pointer, as the compositor numbers it (Linux `BTN_LEFT`). */
const LEFT = 272;

/** Two presses within this many milliseconds on the bar are a double click. */
const DOUBLE_CLICK_MS = 500;

/**
 * The bar. Its width follows the client's, its fill follows the focus and its title the client's title, all read
 * once a frame in `refresh`. A press on a button does what the button says; a press elsewhere on the bar drags the
 * window, and a second press in quick succession maximises or restores it.
 *
 * The drag moves the client by how far the pointer is from where it pressed, in the bar's own coordinates. The bar is
 * a child of the window, so those coordinates stand still while the window follows the pointer.
 */
export class Titlebar extends Box {
    readonly client: TitlebarClient;
    style: TitlebarStyle;
    /** The part of the bar that holds the title: the bar's room that no button takes. */
    readonly dragArea: Box;
    readonly title: Text;
    readonly buttons = new Map<TitlebarButton, Icon>();
    /** The clock the double click is timed by, in milliseconds. */
    now: () => number = () => Date.now();
    #lastPress: number | null = null;
    #dragFrom: { x: number; y: number } | null = null;

    constructor(client: TitlebarClient, style: TitlebarStyle = { ...defaultTitlebarStyle }) {
        super({ direction: "row", height: style.height, padding: style.padding, gap: style.gap, radius: style.radius });
        this.client = client;
        this.style = style;
        this.title = new Text({
            grow: 1,
            size: style.titleSize,
            color: style.titleColor,
            font: style.titleFont,
            ellipsize: true,
        });
        this.dragArea = new Box({ direction: "row", grow: 1, children: [this.title] });
        this.add(this.dragArea);
        for (const name of style.buttons) {
            const button = new Icon({ icon: style.icons[name], size: style.buttonSize, color: style.buttonColor });
            button.onPress = (event) => this.#press(name, event);
            this.buttons.set(name, button);
            this.add(button);
        }
    }

    override refresh(): void {
        const { client, style } = this;
        this.width = client.width;
        this.height = style.height;
        this.padding = style.padding;
        this.gap = style.gap;
        this.radius = style.radius;
        this.fill = client.focused ? style.focusedFill : style.unfocusedFill;
        this.visible = !client.fullscreen;
        this.title.text = client.title;
        this.title.size = style.titleSize;
        this.title.color = style.titleColor;
        for (const [name, button] of this.buttons) {
            button.icon = style.icons[name];
            button.size = style.buttonSize;
            button.color = style.buttonColor;
        }
    }

    #press(name: TitlebarButton, event: UiPointerEvent): void {
        event.stop();
        if (event.button !== LEFT) {
            return;
        }
        if (name === "close") {
            this.client.close();
        } else if (name === "maximise") {
            this.client.maximised = !this.client.maximised;
        } else {
            this.client.minimised = true;
        }
    }

    override onPress(event: UiPointerEvent): void {
        event.stop();
        if (event.button !== LEFT) {
            return;
        }
        const now = this.now();
        const previous = this.#lastPress;
        this.#lastPress = now;
        if (previous !== null && now - previous <= DOUBLE_CLICK_MS) {
            this.#lastPress = null;
            this.client.maximised = !this.client.maximised;
            return;
        }
        this.#dragFrom = { x: event.x, y: event.y };
    }

    override onMotion(event: UiPointerEvent): void {
        const from = this.#dragFrom;
        if (from === null) {
            return;
        }
        event.stop();
        this.client.x += event.x - from.x;
        this.client.y += event.y - from.y;
    }

    override onRelease(event: UiPointerEvent): void {
        if (this.#dragFrom !== null) {
            event.stop();
            this.#dragFrom = null;
        }
    }
}

/** Where a titlebar goes: the node of the window it decorates. */
export type DecorationNode = (client: TitlebarClient) => number;

/**
 * Gives every client one titlebar. It is a collection listener: attach it to `gnoblin.clients` and it decorates each
 * client that is there and each that arrives, and removes the titlebar of one that leaves. When the compositor
 * destroys the window its titlebar's nodes go with it, and the toolkit forgets them.
 */
export class Decorations {
    readonly style: TitlebarStyle;
    readonly #nodeOf: DecorationNode;
    readonly #bars = new Map<TitlebarClient, Titlebar>();

    constructor(nodeOf: DecorationNode, style: TitlebarStyle = { ...defaultTitlebarStyle }) {
        this.#nodeOf = nodeOf;
        this.style = style;
    }

    /** The titlebar of `client`, if it has one. */
    titlebarOf(client: TitlebarClient): Titlebar | undefined {
        return this.#bars.get(client);
    }

    onAdded(client: TitlebarClient): void {
        if (this.#bars.has(client)) {
            return;
        }
        const bar = new Titlebar(client, this.style);
        bar.mount(this.#nodeOf(client));
        this.#bars.set(client, bar);
    }

    onRemoved(client: TitlebarClient): void {
        this.#bars.get(client)?.destroy();
        this.#bars.delete(client);
    }
}
