import type { UiBackend } from "gnoblin-core/host/ui_backend";
import type { Canvas } from "gnoblin-std/ui/canvas";
import type { Text } from "gnoblin-std/ui/text";
/** Receives errors raised while a canvas updates. */
type Report = (site: string, error: unknown) => void;
/** Connects the canvas toolkit to its shell host and error handler. */
export declare function attachUi(host: UiBackend, onError: Report): void;
/** Returns the attached shell host. Throws if no host is attached. */
export declare function host(): UiBackend;
/** Whether a host is attached. */
export declare function attached(): boolean;
/** Sends an error thrown during a canvas update to the handler passed to `attachUi`. */
export declare function reportError(site: string, error: unknown): void;
/** Throws a `GnoblinError` for a host refusal, using `what` as the canvas operation name. */
export declare function refuse(code: string | undefined, what: string): void;
/** Tracks canvases made in the current session, by compositor node id. */
export declare const live: Map<number, Canvas>;
/** Tracks outstanding compositor text measurements by token. */
export declare const measuring: Map<number, {
    text: Text;
    key: string;
}>;
export {};
