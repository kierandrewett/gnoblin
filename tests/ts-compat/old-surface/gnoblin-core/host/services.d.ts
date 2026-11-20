/**
 * Tells `launches` of the windows that appear, take focus or name their application. The listeners are attached
 * before any launch can begin, so the windows that exist now answer none.
 */
export declare function trackLaunches(): void;
/** Makes the dialog, idle, authentication and launch managers. */
export declare function installServices(): void;
