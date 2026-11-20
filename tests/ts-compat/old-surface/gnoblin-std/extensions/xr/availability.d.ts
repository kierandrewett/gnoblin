import type { ControllerTriggerParts } from "gnoblin-core/bindings/trigger_types";
import type { XRController } from "gnoblin-std/extensions/xr/controller";
import type { XRRuntime } from "gnoblin-std/extensions/xr/runtime";
import type { XRExtension, XRListener, XROptions } from "gnoblin-std/extensions/xr/session";
/** Whether `controller` holds exactly the buttons of `trigger`, on its hand. */
export declare function matches(trigger: ControllerTriggerParts, controller: XRController): boolean;
/** The message of an import of an extension that is not there. */
export declare function missingExtension(name: string): string;
/** What `for (const ext of gnoblin.extensions)` yields: the extension's id and where it is in its lifecycle. */
export interface ExtensionInfo {
    /** Extension id, such as `xr`. */
    readonly id: string;
    /** `ready` while the extension serves a session, `unloaded` while it is absent. */
    readonly status: "ready" | "unloaded";
}
/** `gnoblin.extensions`: the optional native extensions, each null while absent. */
export declare class Extensions {
    #private;
    constructor(runtime: XRRuntime);
    /** The XR extension while the program has a session; null otherwise. */
    get xr(): XRExtension | null;
    /**
     * The extensions the shell knows, one `ExtensionInfo` each. The XR extension is `ready` while it has a session
     * (`gnoblin.extensions.xr` is the object) and `unloaded` otherwise (`xr` is null).
     */
    [Symbol.iterator](): Iterator<ExtensionInfo>;
    /** Hears `onSessionStarted(session)` and `onSessionEnded(session)` of the XR extension. */
    addListener(listener: Partial<XRListener>): void;
    /** Stops receiving session lifecycle notifications. */
    removeListener(listener: Partial<XRListener>): void;
    /** What the stock shell does for an extension: `configure("xr", { room: false })` builds no room of its own. */
    configure(name: "xr", options: XROptions): void;
    /** Throws `missingExtension` unless the extension can be used now. `gnoblin-std/extensions/xr` calls it as it loads. */
    require(name: "xr"): void;
}
