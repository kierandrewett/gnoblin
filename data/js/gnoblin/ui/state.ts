// The toolkit's one piece of shared state: the host it is attached to, the error report, the canvases of this
// generation by node and the measurements asked and not yet answered. Every other `gnoblin/ui/*` module reads and
// writes it through the functions here, so there is one owner and no module cycle.

import { GnoblinError, errorCodeOfRefusal } from "gnoblin/errors";
import type { UiBackend } from "gnoblin/ui/backend";
import type { Canvas } from "gnoblin/ui/canvas";
import type { Text } from "gnoblin/ui/text";

type Report = (site: string, error: unknown) => void;

// Set by `attachUi`. Until then making a canvas throws and `runUiFrame` does nothing.
let backend: UiBackend | null = null;
let report: Report = () => {};

/** Connects the toolkit to the shell host. `onError` receives what a canvas threw while it updated. */
export function attachUi(host: UiBackend, onError: Report): void {
    backend = host;
    report = onError;
}

/** The attached host. Throws before `attachUi`. */
export function host(): UiBackend {
    if (backend === null) {
        throw new Error("the UI toolkit needs the shell host and cannot run without it");
    }
    return backend;
}

/** Whether a host is attached. */
export function attached(): boolean {
    return backend !== null;
}

/** Hands an error a canvas threw to the function `attachUi` was given. */
export function reportError(site: string, error: unknown): void {
    report(site, error);
}

/** Throws the host's refusal `code` as a `GnoblinError` that names `Canvas.<what>`; does nothing for `undefined`. */
export function refuse(code: string | undefined, what: string): void {
    if (code !== undefined) {
        throw new GnoblinError(errorCodeOfRefusal(code), `Canvas.${what}`, `the host refused (${code})`);
    }
}

/** The canvases of this generation, by node. */
export const live = new Map<number, Canvas>();

/** The measurements asked and not yet answered, by token. */
export const measuring = new Map<number, { text: Text; key: string }>();
