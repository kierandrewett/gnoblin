# xr-layers

`xr-layers` turns the planned layers of `xr-scene` and the ready swapchains of the runtime into the layers to submit with
`xrEndFrame`. It has no OpenXR types and no GPU code, so every rule runs under `cargo test` with no headset. `xr-session`
turns each `SubmittedLayer` into the `openxr` structs.

## API

- `compose(layers, swapchain, limits) -> Submission`: `layers` is `Plan::layers` in submission order. `swapchain`
  finds the ready swapchain of a View. A projection layer carries its two eyes in the plan. The result holds the layers
  to submit, in plan order, and the `notices`.
- `QuadLayer`: swapchain, space (`World` is the local reference space, `View` the head-locked one), pose, size in metres,
  `EyeVisibility::Both`, the image sub-rectangle, and an optional colour scale.
- `ProjectionLayer`: swapchain, space, and one `ProjectionEye` (pose, field of view, image rectangle) per eye.
- `Limits`: the runtime's `max_layer_count`, the layers the extension reserves, and whether the colour scale extension
  is enabled.

## Rules

- Plan order is submission order. Nothing here reorders a layer or edits a pose or a size.
- A View without a ready swapchain is not submitted (`no-swapchain`) and takes no budget.
- Opacity below 1 becomes the colour scale `[o, o, o, o]` when `XR_KHR_composition_layer_color_scale_bias` is enabled
  (captures are premultiplied). Without it the layer is submitted opaque and reported `opacity-ignored`.
- The budget is `max_layer_count - reserved`. When more layers are ready, `Priority` picks the ones that fit: the View
  that shows a window with keyboard focus first, then the nearest to the head (a world layer is measured from the head's
  position, a head-locked one from the origin of its space), then plan order. The layers that fit are still submitted in
  plan order. Layers that do not fit are dropped and reported `layer-budget`.
- A projection layer reads the left half of the View's image with the left eye and the right half with the right eye,
  which is how the compositor draws a View whose camera is `stereo`. The eyes are the plan's: they come from the camera
  in the tree, not from the runtime.

```sh
cd extensions/gnoblin-xr
cargo test -p xr-layers
```
