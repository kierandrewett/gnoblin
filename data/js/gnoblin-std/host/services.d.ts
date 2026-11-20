import type { DecorationsOptions } from "gnoblin-std/frame/decorations";
/**
 * Gives every window a frame, once: the stock titlebar with its buttons, the drag and the double click, or the frame
 * the options fill. Library code for an `init.ts` that wants frames; a config that does not call it has none. Calling
 * it again changes nothing. It declares to the compositor that this shell draws decorations.
 */
export declare function startDecorations(options?: DecorationsOptions): void;
/** Tells the compositor whether this shell draws window decorations, so a client that asks for server-side is told
 * so. */
export declare function declareDecorations(available: boolean): void;
/** Starts the window menu once: it answers the compositor's menu requests. A config that does not call it has none. */
export declare function startWindowMenu(): void;
/** Starts the not-responding alert once: the dialog for a window that stops answering. */
export declare function startNotRespondingAlert(): void;
/** Attaches the lock screen and the frame host. */
export declare function installServices(): void;
