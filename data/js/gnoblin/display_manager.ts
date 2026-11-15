// `gnoblin/display_manager`: `DisplayManager` reconciles the `OutputProfile`s and `Display`s a script declared with the
// connected outputs. The code is in `gnoblin/display_manager/*`; the host module gives the manager a `DisplayBackend`.

export { DisplayManager } from "gnoblin/display_manager/manager";
export type { DisplayBackend, OutputSet } from "gnoblin/display_manager/backend";
