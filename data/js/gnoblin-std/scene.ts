// `gnoblin-std/scene`, the scene graph: `Node`, `Camera3D`, `View`, `Scene` and the vector and transform types they take.
// The code is in `gnoblin-std/scene/*` (one concept each; `gnoblin-std/scene/registry` owns the node bookkeeping); this module
// is the one import path scripts use, so `Camera3D` and the `Node` it extends are named from the same place.

export { Plane, Ray, Vector3 } from "gnoblin-core/math/vector";
export { Camera3D } from "gnoblin-std/scene/camera";
export { Node } from "gnoblin-std/scene/node";
export { Scene } from "gnoblin-std/scene/scene";
export { Transform3D } from "gnoblin-core/math/transform";
export { View } from "gnoblin-std/scene/view";
export type { Camera3DOptions, Projection } from "gnoblin-std/scene/camera";
export type { InputMode, NodeOptions, NodePointerEvent } from "gnoblin-std/scene/node";
export type { Size2D, ViewOptions } from "gnoblin-std/scene/view";
