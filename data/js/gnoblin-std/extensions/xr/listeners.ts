// The listeners of one XR object: `XRRuntime` (session start and end) and `XRList` (items added and removed).

import { attach, detach } from "gnoblin-core/host/dispatch";

/** The listeners of one XR object, told synchronously; a throw is reported once and the next listener still runs. */
export class XRListeners {
    readonly #list: object[] = [];
    readonly #site: string;
    readonly #report: (site: string, error: unknown) => void;

    /** `site` names the object in a report: a throwing `onAdded` of an `XRList` is reported as `XRList.onAdded`. */
    constructor(site: string, report: (site: string, error: unknown) => void) {
        this.#site = site;
        this.#report = report;
    }

    /** Attaches `listener`; one that is attached already is not attached again. */
    add(listener: object): void {
        if (typeof listener !== "object" || listener === null) {
            throw new TypeError("addListener: the listener must be an object");
        }
        attach(this.#list, listener);
    }

    /** Detaches `listener`; one that is not attached is ignored. */
    remove(listener: object): void {
        detach(this.#list, listener);
    }

    /** Calls `method(argument)` on every listener that has it. */
    tell(method: string, argument: unknown): void {
        for (const listener of [...this.#list]) {
            const handler = Reflect.get(listener, method);
            if (typeof handler === "function") {
                try {
                    handler.call(listener, argument);
                } catch (error) {
                    this.#report(`${this.#site}.${method}`, error);
                }
            }
        }
    }
}
