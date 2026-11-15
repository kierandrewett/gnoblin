// `gnoblin/style`, the style classes and their list elements: `Style`, `ClientStyle`, `Border`, `Shadow` and the
// filters. The code is in `gnoblin/style/*` (one concept each); this module is the one import path scripts use. The
// cascade's plumbing (`bindStyle`, `resolveStyle`, ...) stays in those modules for the host.

export { Border, Shadow } from "gnoblin/style/border_shadow";
export { StyleElement } from "gnoblin/style/element";
export { BlurFilter, BrightnessFilter, ContrastFilter, SaturateFilter } from "gnoblin/style/filters";
export { ClientStyle, Style } from "gnoblin/style/style";
export { MAX_STYLE_LIST } from "gnoblin/style/values";
export type { BorderInit, BorderLike, ShadowInit, ShadowLike } from "gnoblin/style/border_shadow";
export type { Filter, FilterInit } from "gnoblin/style/filters";
export type { StyleList, StyleProperty } from "gnoblin/style/lists";
export type { BorderValue, ComputedStyle, FilterValue, ShadowValue } from "gnoblin/style/resolve";
export type { ClientStyleInit } from "gnoblin/style/style";
export type { CornerRadii } from "gnoblin/style/values";
