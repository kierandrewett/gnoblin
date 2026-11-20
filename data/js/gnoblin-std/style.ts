// `gnoblin-std/style`, the style classes and their list elements: `Style`, `ClientStyle`, `Border`, `Shadow` and the
// filters. The code is in `gnoblin-std/style/*` (one concept each); this module is the one import path scripts use. The
// cascade's plumbing (`bindStyle`, `resolveStyle`, ...) stays in those modules for the host.

export { Border, Shadow } from "gnoblin-std/style/border_shadow";
export { StyleElement } from "gnoblin-std/style/element";
export { BlurFilter, BrightnessFilter, ContrastFilter, SaturateFilter, ShaderFilter } from "gnoblin-std/style/filters";
export { ClientStyle, Style } from "gnoblin-std/style/style";
export { MAX_STYLE_LIST } from "gnoblin-std/style/values";
export type { BorderInit, BorderLike, ShadowInit, ShadowLike } from "gnoblin-std/style/border_shadow";
export type { Filter, FilterInit } from "gnoblin-std/style/filters";
export type { StyleList, StyleProperty } from "gnoblin-std/style/lists";
export type { BorderValue, ComputedStyle, FilterValue, ShadowValue } from "gnoblin-std/style/computed";
export type { ClientStyleInit } from "gnoblin-std/style/style";
export type { CornerRadii } from "gnoblin-std/style/values";
