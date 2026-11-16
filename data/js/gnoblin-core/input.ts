// `gnoblin-core/input`, the input contexts of a seat: `InputContext`, `InputManager` and the shapes of what they carry. The
// code is in `gnoblin-core/input/*` (one concept each); this module is the one import path scripts use. The live lists of
// `gnoblin.input` are `gnoblin-core/input_list` and the device classes `gnoblin-core/devices`.

export { InputContext } from "gnoblin-core/input/context";
export { InputManager } from "gnoblin-core/input/manager";
export type { InputContextOptions } from "gnoblin-core/input/context";
export type { InputHit, InputHost, InputLayout, InputWorld, SeatSettings } from "gnoblin-core/input/host";
export type {
    InputChordMessage,
    InputGestureMessage,
    InputMessage,
    InputPointerMessage,
} from "gnoblin-core/input/messages";
export type { InputBindingInfo } from "gnoblin-core/input/registration";
