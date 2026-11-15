// `gnoblin/scene`, the scene graph: `Node`, `Camera3D`, `View`, `Scene` and the vector and transform types they take.
// The code is in `gnoblin/scene/*` (one concept each; `gnoblin/scene/registry` owns the node bookkeeping); this module
// is the one import path scripts use, so `Camera3D` and the `Node` it extends are named from the same place.

export { Plane, Ray, Vector3 } from "gnoblin/scene/vector";
export { Camera3D } from "gnoblin/scene/camera";
export { Node } from "gnoblin/scene/node";
export { Scene } from "gnoblin/scene/scene";
export { Transform3D } from "gnoblin/scene/transform";
export { View } from "gnoblin/scene/view";
export type { Camera3DOptions, Projection } from "gnoblin/scene/camera";
export type { InputMode, NodeOptions, NodePointerEvent } from "gnoblin/scene/node";
export type { Size2D, ViewOptions } from "gnoblin/scene/view";
