// Shared host connection, error reporting and canvas state for the UI toolkit.

import { GnoblinError, errorCodeOfRefusal } from "gnoblin/errors";
import type { UiBackend } from "gnoblin/ui/backend";
import type { Canvas } from "gnoblin/ui/canvas";
import type { Text } from "gnoblin/ui/text";

/** Receives errors raised while a canvas updates. */
type Report = (site: string, error: unknown) => void;
// Set by `attachUi`; before attachment, canvas creation throws and frame updates do nothing.
let backend: UiBackend | null = null;
let report: Report = () => {};

/** Connects the canvas toolkit to its shell host and error handler. */
export function attachUi(host: UiBackend, onError: Report): void {
    backend = host;
    report = onError;
}

/** Returns the attached shell host. Throws if no host is attached. */
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

/** Sends an error thrown during a canvas update to the handler passed to `attachUi`. */
export function reportError(site: string, error: unknown): void {
    report(site, error);
}

/** Throws a `GnoblinError` for a host refusal, using `what` as the canvas operation name. */
export function refuse(code: string | undefined, what: string): void {
    if (code !== undefined) {
        throw new GnoblinError(errorCodeOfRefusal(code), `Canvas.${what}`, `the host refused (${code})`);
    }
}

/** Tracks canvases made in the current session, by compositor node id. */
export const live = new Map<number, Canvas>();

/** Tracks outstanding compositor text measurements by token. */
export const measuring = new Map<number, { text: Text; key: string }>();
