# xr-scene

`xr-scene` is the pure part of the XR extension: it turns the mirrored node tree into OpenXR layer placements and
picks rays against them. It has no dependencies. It does not open a socket, load a runtime, or touch OpenXR or the GPU,
so every rule here runs under `cargo test` with no headset.

The rules are in `design/node-api-refactor.md`, section 9 (D34). The crate follows them; the module docs in `src/` say
which rule each function implements.

## API

- `TreeView`: the nodes the extension mirrors, keyed by `NodeId`, with the ordered list of roots. Each `TreeNode` has
  its `class` (`Class::View { source, width, height }`, `Class::Camera { projection }`, `Class::Node` or `Class::Other`),
  an optional `name` (the room and the head camera are found by name), ordered `children`, `position`, `rotation` (Euler
  degrees), `scale`, `metres`, `opacity`, `visible` and `input`.
- `plan(tree, head, options) -> Plan`: returns the `layers` to submit, in submission order, and the `skipped` list, each
  entry with its `SkipReason`. The head pose is accepted to match the per-frame call, but it is not read.
- `pick(layers, ray) -> Option<Hit>`: tests a ray against the quad layers in reverse submission order. The first hit wins.
  It returns the View's `id`, its pixel coordinates `(u, v)` within the View's extent, and the hit point.

## Rules

- Only Views under the node named `room` are planned. The room's ancestors contribute transform, opacity and visibility,
  so moving a parent moves the room.
- Views are planned in tree order: depth-first, each node before its children, in child order. That order is the
  submission order. Nothing depth-tests, so the last layer drawn is the one the user sees.
- A visible View with a source becomes a `Quad` in `Space::World`, at its global transform scaled to metres. The
  metres value is the head camera's `metres` if set, else the room's, else `Options::default_metres`.
- A View under the head camera becomes a head-locked `Quad` in `Space::View`. The head camera's own transform and
  everything above it are left out, so the layer moves with the head.
- A View under a stereo camera becomes a `Projection` layer.
- Opacity multiplies down the tree. An invisible node hides its whole subtree.
- `input` is true only when the View and every ancestor accept input. Pick ignores layers with `input` false, so the
  layer underneath can take the ray.
- Poses stay in tracking space. The runtime moves the head, so a world layer does not depend on the head pose.
- A View whose global transform has a NaN or infinity, is singular, or has a non-positive size is skipped with a reason.
  A non-finite or non-positive `metres` value skips every View, also with a reason. The other layers are kept otherwise.

## Determinism

The same tree, head pose and options give an identical `Plan`, compared with `==`. Nothing iterates a hash map.
Float arithmetic is ordinary `f64` in a fixed order.

## Tests

`tests/plan.rs` covers tree order, sibling reordering, nested transform composition, metres scaling, head-locked and
world layers, invisible subtrees, opacity products, stereo projections, unknown classes, determinism and skipped
transforms. `tests/pick.rs` covers the centre of a quad, the nearer-layer rule, misses, rays parallel to the plane and
input-false layers.

```sh
cd extensions/gnoblin-xr
cargo test -p xr-scene
```

## Not here

No OpenXR session, no swapchain, no layer submission and no owner-socket client. Those are the `xr` crate and the
binary, which come later and consume `Plan`.
