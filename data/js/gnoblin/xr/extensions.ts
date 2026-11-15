// Optional native extensions exposed by the shell.

import type { ControllerTriggerParts } from "gnoblin/bindings/trigger_types";
import type { XRController } from "gnoblin/xr/controller";
import { SpatialLayout } from "gnoblin/xr/layout";
import type { XRRuntime } from "gnoblin/xr/runtime";
import type { XRExtension, XRListener, XROptions } from "gnoblin/xr/session";

/** Whether `controller` holds exactly the buttons of `trigger`, on its hand. */
export function matches(trigger: ControllerTriggerParts, controller: XRController): boolean {
    if (trigger.hand !== null && trigger.hand.toLowerCase() !== controller.hand.toLowerCase()) {
        return false;
    }
    return (
        trigger.buttons.length === controller.buttons.size &&
        trigger.buttons.every((button) => controller.buttons.has(button))
    );
}

/** The message of an import of an extension that is not there. */
export function missingExtension(name: string): string {
    return (
        `gnoblin/extensions/${name}: the ${name.toUpperCase()} extension is not available. ` +
        "The session did not declare it (set GNOBLIN_XR=1 and install gnoblin-xr, see docs/xr.md) and no XR program has a session. " +
        `Guard the use with \`gnoblin.extensions.${name}\`, which is null without one, or import the module after the session starts.`
    );
}

/** What `for (const ext of gnoblin.extensions)` yields: the extension's id and where it is in its lifecycle. */
export interface ExtensionInfo {
    /** Extension id, such as `xr`. */
    readonly id: string;
    /** `ready` while the extension serves a session, `unloaded` while it is absent. */
    readonly status: "ready" | "unloaded";
}

/** `gnoblin.extensions`: the optional native extensions, each null while absent. */
export class Extensions {
    readonly #runtime: XRRuntime;

    constructor(runtime: XRRuntime) {
        this.#runtime = runtime;
    }

    /** The XR extension while the program has a session; null otherwise. */
    get xr(): XRExtension | null {
        return this.#runtime.extension;
    }

    /**
     * The extensions the shell knows, one `ExtensionInfo` each. The XR extension is `ready` while it has a session
     * (`gnoblin.extensions.xr` is the object) and `unloaded` otherwise (`xr` is null).
     */
    [Symbol.iterator](): Iterator<ExtensionInfo> {
        const xr: ExtensionInfo = { id: "xr", status: this.#runtime.extension === null ? "unloaded" : "ready" };
        return [Object.freeze(xr)][Symbol.iterator]();
    }

    /** Hears `onSessionStarted(session)` and `onSessionEnded(session)` of the XR extension. */
    addListener(listener: Partial<XRListener>): void {
        this.#runtime.addListener(listener);
    }

    /** Stops receiving session lifecycle notifications. */
    removeListener(listener: Partial<XRListener>): void {
        this.#runtime.removeListener(listener);
    }

    /** What the stock shell does for an extension: `configure("xr", { room: false })` builds no room of its own. */
    configure(name: "xr", options: XROptions): void {
        if (name !== "xr") {
            throw new TypeError(`gnoblin.extensions.configure: unknown extension "${String(name)}"; expected "xr"`);
        }
        if (typeof options !== "object" || options === null) {
            throw new TypeError("gnoblin.extensions.configure: options must be an object");
        }
        for (const key of Object.keys(options)) {
            if (key !== "room" && key !== "layout" && key !== "accept") {
                throw new TypeError(
                    `gnoblin.extensions.configure("xr"): unknown key "${key}"; expected room, layout or accept`,
                );
            }
        }
        if (options.layout !== undefined) {
            new SpatialLayout(options.layout);
        }
        this.#runtime.options = { ...options };
    }

    /** Throws `missingExtension` unless the extension can be used now. `gnoblin/extensions/xr` calls it as it loads. */
    require(name: "xr"): void {
        if (name !== "xr" || !this.#runtime.available()) {
            throw new Error(missingExtension(String(name)));
        }
    }
}
