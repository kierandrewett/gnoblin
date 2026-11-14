// The stock titlebar: a `Box` row of a drag area that holds the title, and the minimise, maximise and close
// buttons, drawn as a child of the window's node (design/archive/node-api-refactor.md, decision 2a).
//
// Everything that sets how it looks is a plain field of `TitlebarStyle`, so a config can hand its own style over
// (`new Titlebar(client, { ...defaultTitlebarStyle, focusedFill: "#223" })`), and a later `defineConfig({ style })`
// can build one from its keys. `Decorations` gives one titlebar to each window the compositor told is server-decorated
// (a window that draws its own title bar gets none) and takes it away when the window leaves or draws its own again.
//
// The bar sits above the window's content, never over it: its y is minus its height, so it lies in the room the layout
// driver keeps free above the client (`Decorations.insetOf`). docs/js-ui.md, "Titlebar", says why.

import { Box, Icon, Text, uiNow, uiSeatAction } from "gnoblin/ui";
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

/** The look of the retired shell's titlebar, with the window buttons beside the close button. */
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
    /** False once the compositor has destroyed the window. A client that does not say is alive. */
    readonly alive?: boolean;
    x: number;
    y: number;
    maximised: boolean;
    minimised: boolean;
    close(): void;
    /** Moves `opacity` and `scale` over time: the fade of a window that is closing. Without it the window just closes. */
    animate?(
        values: { opacity?: number; scale?: [number, number, number] },
        options: { duration: number; easing?: string },
    ): void;
}

/** What the seat's click actions do that the titlebar cannot do alone. The host supplies them. */
export interface TitlebarHooks {
    /** The seat's `menu` action: open the window menu where the pointer is. */
    menu?(client: TitlebarClient): void;
    /** The seat's `lower` action: put the window under the others. */
    lower?(client: TitlebarClient): void;
    /** Maximises or restores the window as the shell does it, which leaves room for the bar. Else the flag is written. */
    maximise?(client: TitlebarClient, on: boolean): void;
    /** Minimises the window as the shell does it, which hands the keyboard focus to another window. Else the flag is written. */
    minimise?(client: TitlebarClient): void;
    /** The seconds an animation takes: 0 while the seat asks for reduced motion. Absent, the seconds it was given. */
    motion?(seconds: number): number;
    /** A button went down on the bar itself (not on one of its buttons): the window is pressed, so it takes the focus. */
    press?(client: TitlebarClient, event: { x: number; y: number; button: number }): void;
    /** The button that went down on the bar came up. */
    release?(client: TitlebarClient, event: { x: number; y: number; button: number }): void;
}

/** The buttons of the pointer, as the compositor numbers them (Linux `BTN_LEFT`, `BTN_RIGHT`, `BTN_MIDDLE`). */
const LEFT = 272;
const RIGHT = 273;
const MIDDLE = 274;

/** Two presses within this many milliseconds on the bar are a double click. */
const DOUBLE_CLICK_MS = 500;

/** The retired shell's fade on close: out over 0.18 s, as a window shrinks to 0.92 of its size. */
export const CLOSE_FADE_MS = 180;
/** A window that is still there this long after it was asked to close (a client that asks "save changes?") comes back. */
export const CLOSE_RESTORE_MS = 1500;
const RESTORE_SECONDS = 0.12;

/** What the seat's property says each click does when the tree has no seat to ask: its schema default. */
const DEFAULT_ACTIONS = { double_click: "toggle-maximize", middle_click: "lower", right_click: "menu" } as const;

type Gesture = keyof typeof DEFAULT_ACTIONS;

/**
 * The bar. Its width follows the client's, its fill follows the focus and its title the client's title, all read
 * once a frame in `refresh`. A press on a button does what the button says; a press elsewhere on the bar drags the
 * window, and a second press in quick succession, a middle click and a right click do what the seat's `double_click`,
 * `middle_click` and `right_click` say (`toggle-maximize`, `lower`, `menu`, `minimize` or `none`).
 *
 * The close button fades the window out first (`CLOSE_FADE_MS`) and then closes it; a window that is still there
 * `CLOSE_RESTORE_MS` later is shown again, as a client that asked the user something is. A window the client closes
 * itself is gone from the compositor at once, so there is nothing to fade.
 *
 * The drag moves the client by how far the pointer is from where it pressed, in the bar's own coordinates. The bar is
 * a child of the window, so those coordinates stand still while the window follows the pointer.
 */
export class Titlebar extends Box {
    readonly client: TitlebarClient;
    style: TitlebarStyle;
    readonly hooks: TitlebarHooks;
    /** The part of the bar that holds the title: the bar's room that no button takes. */
    readonly dragArea: Box;
    readonly title: Text;
    readonly buttons = new Map<TitlebarButton, Icon>();
    /** The clock the double click and the fade are timed by, in milliseconds. */
    now: () => number = () => uiNow();
    #lastPress: number | null = null;
    #dragFrom: { x: number; y: number } | null = null;
    /** The fade of a window being closed: when to close it, and when to bring it back if it stayed. */
    #closing: { closeAt: number | null; restoreAt: number | null } | null = null;

    constructor(client: TitlebarClient, style: TitlebarStyle = { ...defaultTitlebarStyle }, hooks: TitlebarHooks = {}) {
        super({ direction: "row", height: style.height, padding: style.padding, gap: style.gap, radius: style.radius });
        this.client = client;
        this.style = style;
        this.hooks = hooks;
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

    /** The room the bar takes above the client: its height, or nothing while the window is fullscreen and the bar hidden. */
    get inset(): number {
        return this.client.fullscreen ? 0 : this.style.height;
    }

    override refresh(): void {
        const { client, style } = this;
        this.width = client.width;
        this.height = style.height;
        // Above the content: the bar is a child of the window node, whose origin is the content's corner.
        this.x = 0;
        this.y = -style.height;
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
        this.#advanceClose();
    }

    /** Starts the fade out of a window that is being closed. A second press while it fades does nothing. */
    #startClose(): void {
        if (this.#closing !== null) {
            return;
        }
        const { client } = this;
        if (client.animate === undefined) {
            client.close();
            return;
        }
        const fade = this.hooks.motion?.(CLOSE_FADE_MS / 1000) ?? CLOSE_FADE_MS / 1000;
        client.animate({ opacity: 0, scale: [0.92, 0.92, 1] }, { duration: fade, easing: "in-cubic" });
        if (fade === 0) {
            // Reduced motion: nothing to wait for, so the window is asked to close now, and comes back as usual if it stays.
            this.#closing = { closeAt: null, restoreAt: this.now() + CLOSE_RESTORE_MS };
            client.close();
            return;
        }
        this.#closing = { closeAt: this.now() + fade * 1000, restoreAt: null };
    }

    #advanceClose(): void {
        const closing = this.#closing;
        const { client } = this;
        if (closing === null) {
            return;
        }
        if (client.alive === false) {
            this.#closing = null;
            return;
        }
        const now = this.now();
        if (closing.closeAt !== null && now >= closing.closeAt) {
            closing.closeAt = null;
            closing.restoreAt = now + CLOSE_RESTORE_MS;
            client.close();
        } else if (closing.restoreAt !== null && now >= closing.restoreAt) {
            this.#closing = null;
            client.animate?.({ opacity: 1, scale: [1, 1, 1] }, { duration: RESTORE_SECONDS, easing: "out" });
        }
    }

    #press(name: TitlebarButton, event: UiPointerEvent): void {
        event.stop();
        if (event.button !== LEFT) {
            return;
        }
        if (name === "close") {
            this.#startClose();
        } else if (name === "maximise") {
            this.#toggleMaximise();
        } else {
            this.#minimise();
        }
    }

    #toggleMaximise(): void {
        const { client } = this;
        const on = !client.maximised;
        if (this.hooks.maximise === undefined) {
            client.maximised = on;
        } else {
            this.hooks.maximise(client, on);
        }
    }

    #minimise(): void {
        const { client } = this;
        if (this.hooks.minimise === undefined) {
            client.minimised = true;
        } else {
            this.hooks.minimise(client);
        }
    }

    /** Runs what the seat says a click on the bar does. An action the schema does not name is an error. */
    #gesture(property: Gesture): void {
        const { client } = this;
        const action = uiSeatAction(property) ?? DEFAULT_ACTIONS[property];
        switch (action) {
            case "toggle-maximize":
                this.#toggleMaximise();
                return;
            case "lower":
                this.hooks.lower?.(client);
                return;
            case "menu":
                this.hooks.menu?.(client);
                return;
            case "minimize":
                this.#minimise();
                return;
            // Maximising along one axis has no stock behaviour yet, as in the retired shell: it does nothing.
            case "toggle-maximize-horizontally":
            case "toggle-maximize-vertically":
            case "none":
                return;
            default:
                throw new Error(`${property} must name a titlebar action, got ${action}`);
        }
    }

    override onPress(event: UiPointerEvent): void {
        event.stop();
        // A left press lands on the window: it takes the focus and, as the seat says, the top. A right or middle press is
        // left to its action: the window menu a right click opens holds the keyboard, and the press is heard later.
        if (event.button === LEFT) {
            this.hooks.press?.(this.client, { x: event.x, y: event.y, button: event.button });
        }
        if (event.button === MIDDLE) {
            this.#gesture("middle_click");
            return;
        }
        if (event.button === RIGHT) {
            this.#gesture("right_click");
            return;
        }
        if (event.button !== LEFT) {
            return;
        }
        const now = this.now();
        const previous = this.#lastPress;
        this.#lastPress = now;
        if (previous !== null && now - previous <= DOUBLE_CLICK_MS) {
            this.#lastPress = null;
            this.#gesture("double_click");
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
        this.hooks.release?.(this.client, { x: event.x, y: event.y, button: event.button });
        if (this.#dragFrom !== null) {
            event.stop();
            this.#dragFrom = null;
        }
    }
}

/** A client the decorations can ask who draws its frame. A `Client` of the host module is one. */
export interface DecoratedClient extends TitlebarClient {
    /**
     * Who draws the window's frame, as the compositor negotiated it (`Window.decoration`): `server` when the shell
     * does, `client` when the client draws its own title bar, `none` for a window nobody frames.
     */
    readonly decoration: string;
}

/** Where a titlebar goes: the node of the window it decorates. */
export type DecorationNode = (client: DecoratedClient) => number;

/**
 * Gives every window the compositor told to be server-decorated one titlebar. It is a collection listener: attach it to
 * `gnoblin.clients` and it decorates each such client that is there and each that arrives, takes the titlebar away from
 * one whose decoration changes to something else, and removes the titlebar of one that leaves. When the compositor
 * destroys the window its titlebar's nodes go with it, and the toolkit forgets them.
 *
 * A client that draws its own title bar (a GTK or Qt window, foot) is `client` and gets none: a second bar over it
 * would be the double title bar this rule exists to prevent. The compositor tells a client that asks for server-side
 * that the server draws only while this shell has declared decorations (`Seat.decorations_available`), so a window
 * that asked for a frame the shell does not draw is told to draw its own.
 *
 * `insetOf(client)` is how much room the client's bar takes above its content: 0 for a client with no bar. The layout
 * driver leaves that much free above every window it places, so the bar and the content never overlap.
 */
export class Decorations {
    readonly style: TitlebarStyle;
    readonly hooks: TitlebarHooks;
    readonly #nodeOf: DecorationNode;
    readonly #bars = new Map<DecoratedClient, Titlebar>();

    constructor(nodeOf: DecorationNode, style: TitlebarStyle = { ...defaultTitlebarStyle }, hooks: TitlebarHooks = {}) {
        this.#nodeOf = nodeOf;
        this.style = style;
        this.hooks = hooks;
    }

    /** The titlebar of `client`, if it has one. */
    titlebarOf(client: DecoratedClient): Titlebar | undefined {
        return this.#bars.get(client);
    }

    /** The height of the bar above `client`'s content: 0 for a client with no bar, or one that is fullscreen. */
    insetOf(client: DecoratedClient): number {
        return this.#bars.get(client)?.inset ?? 0;
    }

    onAdded(client: DecoratedClient): void {
        this.#follow(client);
    }

    /** The compositor changed who draws the client's frame: the bar comes or goes with it. */
    onDecorationChanged(client: DecoratedClient): void {
        this.#follow(client);
    }

    onRemoved(client: DecoratedClient): void {
        this.#bars.get(client)?.destroy();
        this.#bars.delete(client);
    }

    /** Gives the client a bar when it is server-decorated and has none, and takes it away when it is not. */
    #follow(client: DecoratedClient): void {
        const bar = this.#bars.get(client);
        if (client.decoration !== "server") {
            if (bar !== undefined) {
                bar.destroy();
                this.#bars.delete(client);
            }
            return;
        }
        if (bar === undefined) {
            const made = new Titlebar(client, this.style, this.hooks);
            made.mount(this.#nodeOf(client));
            this.#bars.set(client, made);
        }
    }
}
