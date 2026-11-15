// `gnoblin/ui`, the shell UI toolkit: `Canvas`, `Box`, `Text`, `Icon` and `Grid`, the classes the stock shell draws
// with. The code is in `gnoblin/ui/*` (one concept each; `gnoblin/ui/state` owns the shared state); this module is the
// one import path scripts use. Read `gnoblin/ui/canvas` for how a frame reaches the compositor.

export type { UiBackend, UiAnswer, UiRawEvent } from "gnoblin/ui/backend";
export { attachUi } from "gnoblin/ui/state";
export { utf8, encodePaintList } from "gnoblin/ui/paint";
export type { Color, PaintOp } from "gnoblin/ui/paint";
export { UiPointerEvent, UiKeyEvent } from "gnoblin/ui/events";
export { Canvas } from "gnoblin/ui/canvas";
export type { CanvasPlacement, CanvasOptions } from "gnoblin/ui/canvas";
export { Box } from "gnoblin/ui/box";
export type { Direction, BoxOptions } from "gnoblin/ui/box";
export { estimateText, Text } from "gnoblin/ui/text";
export type { TextMetrics, TextOptions } from "gnoblin/ui/text";
export { Icon } from "gnoblin/ui/icon";
export type { IconOptions } from "gnoblin/ui/icon";
export { Grid } from "gnoblin/ui/grid";
export { uiNodeGone, uiReleaseAll, uiCanvasCount, onUiRequest, onUiFrame, runUiFrame } from "gnoblin/ui/frame";
export type { UiRequest } from "gnoblin/ui/frame";
export { uiNow, uiFocused, uiFocus, uiLocked, uiScreen, uiPointer, uiSeatAction } from "gnoblin/ui/session";
