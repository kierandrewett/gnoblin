// The shape of the shell host the UI toolkit runs on: `UiBackend`, and the two kinds of message that come back from
// it. The toolkit never imports the host, so a test can attach a fake with `attachUi`.

/** The functions the shell host provides. Each refusal is a short code (`unknown-node`, `not-owned`, ...). */
export interface UiBackend {
    /** Makes a canvas node in the tree and returns its id, or `undefined` when the shell cannot make one. */
    create(): number | undefined;
    insert(node: number, parent: number, index: number): string | undefined;
    destroy(node: number): string | undefined;
    set(node: number, key: string, value: number | boolean | string): string | undefined;
    /** Writes the encoded paint list as the node's `content`. */
    paint(node: number, bytes: number[]): string | undefined;
    /** Asks the compositor how much room `text` takes. The answer comes back from `takeAnswers` under the token. */
    measure(node: number, font: string, size: number, text: string): number | undefined;
    takeAnswers(): UiAnswer[];
    takeEvents(): UiRawEvent[];
    /** Gives the seat's keyboard focus to a canvas or a window (`0` takes it away). A refusal is a short code. */
    focus(node: number): string | undefined;
    /** The node with the keyboard focus, or `undefined`. */
    focused(): number | undefined;
    /** Puts a canvas last under the root, over every window. */
    top(node: number): string | undefined;
    /** A number the seat publishes: `pointer_x`, `pointer_y`. `undefined` when the tree has no seat. */
    seatNumber(key: string): number | undefined;
    /** What a titlebar click does: the seat's `double_click`, `middle_click` or `right_click`. */
    seatText(key: string): string | undefined;
    /** Whether the session is locked or about to be. */
    locked(): boolean;
    /** The size of the primary screen, or `undefined` while the tree has none. */
    screen(): [number, number] | undefined;
    /** Milliseconds on a clock that only goes forward. */
    now(): number;
}

/** The compositor's answer to a measurement: `result` is `<width> <height> <ascent>` in whole pixels. */
export interface UiAnswer {
    token: number;
    result: string;
    error: string;
}

/**
 * What the compositor sent to the toolkit before it is routed: a pointer event that hit a canvas, a key sent to the
 * canvas that holds the keyboard focus, or a request made of a window (`request` is `menu` for its window menu, and
 * `node` is the window).
 */
export interface UiRawEvent {
    node: number;
    kind: string;
    x: number;
    y: number;
    button: number;
    dx: number;
    dy: number;
    key: string;
    text: string;
    pressed: boolean;
    request: string;
}
