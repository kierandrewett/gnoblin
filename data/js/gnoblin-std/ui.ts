// The script-facing import path for Gnoblin's UI toolkit: Canvas, Box, Text, Icon and Grid.
// The toolkit draws shell UI using the canvas and compositor paint operations.

export type { UiBackend, UiAnswer, UiRawEvent } from "gnoblin-std/ui/backend";
export { attachUi } from "gnoblin-std/ui/state";
export { utf8, encodePaintList } from "gnoblin-std/ui/paint";
export type { Color, PaintOp } from "gnoblin-std/ui/paint";
export { UiPointerEvent, UiKeyEvent } from "gnoblin-std/ui/events";
export { Canvas } from "gnoblin-std/ui/canvas";
export type { CanvasPlacement, CanvasOptions } from "gnoblin-std/ui/canvas";
export { Box } from "gnoblin-std/ui/box";
export type { Direction, BoxOptions } from "gnoblin-std/ui/box";
export { estimateText, Text } from "gnoblin-std/ui/text";
export type { TextMetrics, TextOptions } from "gnoblin-std/ui/text";
export { Icon } from "gnoblin-std/ui/icon";
export type { IconOptions } from "gnoblin-std/ui/icon";
export { Grid } from "gnoblin-std/ui/grid";
export { uiNodeGone, uiReleaseAll, uiCanvasCount, onUiRequest, onUiFrame, runUiFrame } from "gnoblin-std/ui/frame";
export type { UiRequest } from "gnoblin-std/ui/frame";
export { uiNow, uiFocused, uiFocus, uiLocked, uiScreen, uiPointer, uiSeatAction } from "gnoblin-std/ui/session";
