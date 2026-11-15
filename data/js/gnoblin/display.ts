// `gnoblin/display`, logical displays (owner-spec section 9, cameras in section 8): `OutputProfile`, `Display`,
// `Viewport` and `Camera2D`. The code is in `gnoblin/display/*` (one concept each; `gnoblin/display/state` owns the
// shell's side); this module is the one import path they share. What a display does to the hardware and to the desks is
// `DisplayManager` (`gnoblin/display_manager`), which reads the state kept here through the `*State` accessors.

export { supersampleNotice, outputRequest } from "gnoblin/display/output_config";
export type { OutputTransform, OutputConfig, OutputSetting } from "gnoblin/display/output_config";
export type {
    DisplayOutputCollection,
    DisplayVec2,
    OutputMatch,
    OutputProfileMode,
    OutputAlign,
    OutputProfilePosition,
    PlacementTarget,
    OutputView,
    OutputProfileInit,
    DisplayModeName,
    DisplayInit,
    ViewportInit,
    Camera2DInit,
} from "gnoblin/display/types";
export { attachDisplays } from "gnoblin/display/state";
export type { DisplayHooks } from "gnoblin/display/state";
export {
    profileState,
    OutputProfile,
    relativeTarget,
    serialOf,
    profileMatches,
    profileSetting,
} from "gnoblin/display/output_profile";
export type { ProfileState } from "gnoblin/display/output_profile";
export { Camera2D, isIdentity } from "gnoblin/display/camera2d";
export { viewportState, Viewport } from "gnoblin/display/viewport";
export type { ViewportState } from "gnoblin/display/viewport";
export { displayState, Display, implicitDisplay } from "gnoblin/display/display";
export type { DisplayState } from "gnoblin/display/display";
