/** The listeners of one XR object, told synchronously; a throw is reported once and the next listener still runs. */
export declare class XRListeners {
    #private;
    /** `site` names the object in a report: a throwing `onAdded` of an `XRList` is reported as `XRList.onAdded`. */
    constructor(site: string, report: (site: string, error: unknown) => void);
    /** Attaches `listener`; one that is attached already is not attached again. */
    add(listener: object): void;
    /** Detaches `listener`; one that is not attached is ignored. */
    remove(listener: object): void;
    /** Calls `method(argument)` on every listener that has it. */
    tell(method: string, argument: unknown): void;
}
