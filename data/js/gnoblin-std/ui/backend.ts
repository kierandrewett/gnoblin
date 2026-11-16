// The interface between the canvas toolkit and its shell host, including compositor events and replies.

/** The functions the shell host provides. Each refusal is a short code (`unknown-node`, `not-owned`, ...). */
export interface UiBackend {
    /** Makes a canvas node in the tree and returns its id, or `undefined` when the shell cannot make one. */
    create(): number | undefined;
    /** Inserts a node under `parent` at `index`; returns a refusal code when the operation fails. */
    insert(node: number, parent: number, index: number): string | undefined;
    /** Destroys a node and returns a refusal code if the host cannot destroy it. */
    destroy(node: number): string | undefined;
    /** Sets a node property and returns a refusal code if the host rejects it. */
    set(node: number, key: string, value: number | boolean | string): string | undefined;
    /** Writes the encoded paint list as the node's `content`. */
    paint(node: number, bytes: number[]): string | undefined;
    /** Requests text metrics and returns the token used to match its answer. */
    measure(node: number, font: string, size: number, text: string): number | undefined;
    /** Takes pending measurement answers from the host. */
    takeAnswers(): UiAnswer[];
    /** Takes pending compositor events from the host. */
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

/** A compositor reply to a text measurement request. */
export interface UiAnswer {
    /** The request token returned by `measure`. */
    token: number;
    /** A space-separated width, height and ascent in logical pixels. */
    result: string;
    /** The measurement error, if any. */
    error: string;
}

/**
 * An event from the compositor before routing. Pointer events target a canvas, key events target the focused canvas,
 * and window requests include the window node.
 */
export interface UiRawEvent {
    /** The target node: a canvas for input, or a window for a request. */
    node: number;
    /** The event kind, such as `press`, `key` or `request`. */
    kind: string;
    /** Horizontal event or request position in logical pixels. */
    x: number;
    /** Vertical event or request position in logical pixels. */
    y: number;
    /** The pointer button number. */
    button: number;
    /** Horizontal movement or scroll distance in logical pixels. */
    dx: number;
    /** Vertical movement or scroll distance in logical pixels. */
    dy: number;
    /** The xkbcommon keysym name for a key event. */
    key: string;
    /** Typed text for a key event, or empty when no text is produced. */
    text: string;
    /** Whether the key is pressed rather than released. */
    pressed: boolean;
    /** The window request kind, such as `menu`. */
    request: string;
}
