# Lua model for spatial outputs

**Status:** proposal. No XR output, spatial scene, or API in this document is implemented. Names and shapes below are proposals, including the LuaCATS annotations.

Gnoblin should represent a headset as an output and let Lua own spatial policy for that output. Lua decides which surfaces belong in the scene, where they go, how users manipulate them, and what placement survives a restart.

Gnoblin's C compositor path owns XR session and frame timing, tracking data, surface import, projection, hit testing, input delivery, transform application, and failure recovery. It must never call Lua on the render or tracking thread. The headset runtime presents the rendered view; Lua does not draw or submit frames.

This changes the ownership statement in `design/outputs.md`, which currently reserves `kind = "xr"` but says the shell owns spatial placement. The existing Lua API keeps presentation in Wayland clients and exposes typed compositor state and operations. Layer-shell and toplevel surfaces remain client surfaces; Lua controls their spatial compositor state through Gnoblin.

## Evidence and limits

The repository's `design/outputs.md` says output records have stable connector IDs and that an XR output has per-eye views without a logical desktop rectangle. XR and virtual outputs are reserved, not implemented. `src/config/lua/api/monitors-layers.h` documents current monitor and layer snapshots; there is no output or spatial Lua header. `windows.h` provides logical-pixel movement and resize operations, and event contexts provide trusted interaction authority.

`design/lua-input-policy.md` requires compositor-side filtering, bounded queues, no waiting for Lua, and safe event retirement on timeout or worker failure. `design/lua-api.md` specifies that Lua callbacks run on the supervised event loop and compositor mutations are asynchronous operations.

These public sources establish adjacent protocol facts, not a shipped Gnoblin XR implementation:

- The [OpenXR specification](https://registry.khronos.org/OpenXR/specs/1.1/html/xrspec.html)
  defines application-facing XR sessions, spaces, views, and frame progression. It does not define a Wayland compositor output for a headset.
- The [Wayland protocols repository](https://gitlab.freedesktop.org/wayland/wayland-protocols/-/blob/main/staging/wlr-layer-shell/wlr-layer-shell-unstable-v1.xml)
  defines layer-shell surface roles and anchoring against a 2D output area. It has no metre-based pose, curvature, or XR docking contract.
- The [Wayland core protocol](https://gitlab.freedesktop.org/wayland/wayland/-/blob/main/protocol/wayland.xml)
  defines pointer enter, motion, button, and axis events in surface-oriented coordinates. It does not specify controller rays or hand gestures.

The Gnoblin XR runtime boundary remains to be selected. This proposal does not assume Mutter currently exposes an OpenXR-backed `MetaMonitor`, that an OpenXR runtime can be loaded into the compositor, or that an existing Wayland extension carries these objects. A vertical slice must validate the headset and runtime integration before these APIs can be implemented.

## Output and scene model

`gnoblin.outputs.list()`, `primary()`, and `by_id(id)` remain the output entry points proposed by `design/outputs.md`. An XR output has `kind = "xr"`, a stable output `id`, renderer information, and an `xr` descriptor.

Its physical mode may describe recommended per-eye render size, but `x` and `y` are absent: it does not consume a rectangle in the desktop layout and is not a monitor selector for 2D `Window:move()`.

The C side creates or discovers the output after its XR session is available. It reports lifecycle and capability changes as events. Lua handles those events, creates or updates a spatial scene, and issues typed operations.

When the runtime is absent, suspended, or loses tracking, Lua sees a state change; C retains a safe compositor state and retires stale interaction contexts. It does not synthesize a fake display output.

An XR output owns one scene. The scene contains:

| Object           | Meaning                                                                                                                                              |
| ---------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------- |
| `Space`          | Named coordinate frame. The initial proposal has `world` (stable to the room/runtime reference) and `view` (moves with the head).                    |
| `Anchor`         | Pose in a `Space`, with stable ID and optional persistence key; parents objects placed relative to a point.                                          |
| `Viewport`       | User-visible frame in the world. It has a pose, physical dimensions in metres, and optional curvature. Moving it moves its pill and docked surfaces. |
| `Pill`           | Draggable handle associated with a viewport. It is a compositor-managed hit target and visual affordance; its artwork is a normal client surface.    |
| `SpatialSurface` | Toplevel or layer-shell surface with pose, physical dimensions, curvature, parent relation, visibility, and spatial stacking rank.                   |

Positions and sizes use metres. Use the OpenXR right-handed convention: +X is right, +Y is up, and -Z is forward. A pose is relative to a named space or parent and consists of translation plus a unit quaternion. Rotation uses quaternions to avoid Euler-order ambiguity. Size is the visible panel size, independent of client logical buffer dimensions.

Curvature is an optional signed angle in radians across the surface's horizontal span; zero is flat. C validates finite values, bounds, normalized rotations, and supported dimensions before applying an operation.

An object is either docked to a parent or placed freely under an anchor. Docked surfaces store a local pose relative to the viewport, so they follow it without Lua resending transforms each frame. Free placement stores an anchor-relative pose and is unaffected by viewport motion. Reparenting preserves the current world pose unless Lua explicitly supplies a new local pose.

Removing a viewport must detach its children to their last world pose or reject removal while children remain; implementation must choose and document one rule.

Layer-shell's existing 2D `anchor`, `exclusive_zone`, and output fields keep their current meaning on ordinary outputs. They do not implicitly become 3D anchors. For XR, Gnoblin treats the layer surface as a surface identity and Lua explicitly attaches it to a viewport or spatial anchor.

A shell may use a layer surface as pill artwork, but the pill's hit target and drag behavior belong to Gnoblin so policy does not depend on a shell-specific widget.

## Input, focus, and stacking

The headset runtime produces controller and hand tracking samples in C. C projects a controller ray or recognized hand pointer into the current scene, hit-tests using the transform revision used for rendering, and emits typed spatial pointer events to Lua. Events include source, phase, target ID, surface-local coordinates when hit, monotonic timestamp, tracking validity, and an opaque one-use context.

That context authorizes resulting focus, pointer, or drag operations; scripts cannot forge it from a table.

For compatibility, a pointer aimed at a Wayland surface becomes the usual Wayland pointer enter/motion/button stream with surface-local coordinates. Controller identity and gesture metadata remain in Gnoblin's typed events; they are not encoded as fake keyboard or mouse devices. C owns pointer focus, button state, drag capture, cancellation on tracking loss, and exactly-once delivery.

Lua chooses policy in callbacks, such as whether a spatial event starts a viewport drag or window operation.

Initial gestures are explicit policy, not protocol names:

- **Point:** ray motion updates hover and pointer position on the hit surface.
- **Click:** a controller trigger or supported hand pinch emits a
  pointer press/release pair. C retains release ownership if Lua consumes the press.
- **Corner resize:** a drag beginning on a registered corner handle resizes
  the target panel while preserving its pose and opposite corner. Client content size, compositor panel size, and pose are distinct. Client resize configure is asynchronous; C reports accepted geometry and does not present a stale buffer as if it were accepted.
- **Menu plus motion:** while the controller menu button is held, C captures
  the controller pose as drag origin. Motion applies its pose delta to the selected target. Releasing menu commits the last pose; tracking loss, device removal, target closure, or session lock cancels and releases captured input.

Spatial focus is one active keyboard/text-input target for the XR seat, independent of hover. Clicking a focusable window can request activation with the input context; C applies Mutter activation rules. Hover alone never activates. Modal/transient surfaces remain above their parent. Lua sets other surfaces' stacking order through a typed operation; C resolves deterministic draw and hit-test order.

Focus raises a surface only if policy requests it. A session lock, exclusive input capture, or tracking loss overrides ordinary Lua policy.

## Callbacks, frame work, animation, and persistence

Most Lua work is event-driven: output lifecycle, tracking state, surface map and unmap, pointer enter/move/press/release, gesture begin/update/end/cancel, focus, and animation completion. Callbacks run on the supervised event loop and use non-blocking typed operations.

Lua should not run on every XR frame. A proposed opt-in `on_frame` callback is enabled only while a scene requests continuous policy work. C samples latest state, invokes it at a bounded cadence (proposed maximum 30 Hz), coalesces intermediate updates, and limits each callback's snapshot size, operation count, and wall time.

On budget exhaustion, it drops the tick, records a diagnostic, and renders from the last accepted state. This callback is for small policy updates, not tracking, raycasting, physics, interpolation, per-pixel work, blocking I/O, or frame submission. With no subscriber or active animation, no tick runs. Cadence and budget require target-hardware profiling before becoming a contract.

C performs animation interpolation against the compositor monotonic frame clock and applies transforms at presentation time. Lua supplies target values, duration, and easing, and receives start/finish/cancel events. Animation stays smooth if Lua is busy or restarting. An interrupted animation starts its replacement from the currently presented transform. The compositor may late-latch head pose independently of Lua scene transforms.

Lua chooses which objects and fields to persist and assigns stable placement keys (for example, application ID plus role, with a user override for ambiguous windows). Gnoblin stores a versioned, size-limited, validated snapshot under `$XDG_STATE_HOME/gnoblin/`, writes atomically, and does not serialize runtime handles, controller identities, input contexts, titles by default, or tracking samples.

On startup Lua matches new surfaces against saved keys and restores free or docked poses. Missing parents, unsupported schema versions, malformed values, and stale output IDs fall back to defaults with a diagnostic. Path, retention, and user controls remain open decisions.

## Proposed LuaCATS surface

Every field and function below is proposed. The block follows `src/config/lua/api/*.h`; it is a design sketch, not an implementation claim. Existing `Window`, `LayerSurface`, `Operation`, and event types are reused where suitable, but the new spatial methods and events do not exist today.

```lua
---@class XrView [proposed]
---@field eye "left"|"right" Proposed eye identifier for a per-eye view.
---@field width integer Proposed recommended render width in pixels.
---@field height integer Proposed recommended render height in pixels.

---@class XrOutputInfo [proposed]
---@field state "ready"|"suspended"|"unavailable" Proposed session state.
---@field views XrView[] Proposed current per-eye views.
---@field tracking_valid boolean Proposed indication that head pose is usable.

---@class Output [proposed]
---@field id string Proposed stable output ID.
---@field kind "display"|"virtual"|"xr" Proposed output kind; virtual and XR remain reserved.
---@field name? string Proposed output name when supplied.
---@field make? string Proposed manufacturer when supplied.
---@field model? string Proposed model when supplied.
---@field serial? string Proposed serial number when supplied.
---@field enabled boolean Proposed enabled state.
---@field primary boolean Proposed primary-output state.
---@field monitor_id? string Proposed related 2D monitor ID, when applicable.
---@field mode? {width: integer, height: integer, refresh_rate: number} Proposed current physical mode or XR recommended render mode.
---@field scale? number Proposed display scale when applicable.
---@field transform? string Proposed 2D display transform when applicable.
---@field renderer? {name: string, vendor?: string, api: string, accelerated: boolean} Proposed renderer snapshot.
---@field x? integer Proposed logical x; omitted for XR outputs.
---@field y? integer Proposed logical y; omitted for XR outputs.
---@field xr? XrOutputInfo Proposed XR state, present only for kind "xr".
---@field revision integer Proposed snapshot revision.

---@class Vector3 [proposed]
---@field x number Proposed finite coordinate in metres.
---@field y number Proposed finite coordinate in metres.
---@field z number Proposed finite coordinate in metres.

---@class Quaternion [proposed]
---@field x number Proposed quaternion component.
---@field y number Proposed quaternion component.
---@field z number Proposed quaternion component.
---@field w number Proposed quaternion component; quaternion must be normalized.

---@class Pose [proposed]
---@field position Vector3 Proposed translation relative to the declared space or parent.
---@field rotation Quaternion Proposed orientation relative to the declared space or parent.

---@class SpatialSize [proposed]
---@field width number Proposed visible width in metres.
---@field height number Proposed visible height in metres.

---@class SpatialCurvature [proposed]
---@field angle number Proposed signed horizontal curvature in radians; zero is flat.

---@class Space [proposed]
---@field id string Proposed stable space ID.
---@field kind "world"|"view" Proposed room-fixed or head-relative frame.
---@field revision integer Proposed state revision.

---@class Anchor [proposed]
---@field id string Proposed stable anchor ID.
---@field space_id string Proposed parent space ID.
---@field pose Pose Proposed anchor pose.
---@field persistence_key? string Proposed key for persistence matching.

---@class Viewport [proposed]
---@field id string Proposed stable viewport ID.
---@field output_id string Proposed XR output ID.
---@field anchor_id string Proposed parent anchor ID.
---@field pose Pose Proposed pose relative to its anchor.
---@field size SpatialSize Proposed physical dimensions in metres.
---@field curvature? SpatialCurvature Proposed optional horizontal curvature.
---@field pill_id? string Proposed associated pill ID.

---@class Pill [proposed]
---@field id string Proposed stable pill ID.
---@field viewport_id string Proposed viewport moved by this pill.
---@field surface_id string Proposed client surface used for pill artwork.
---@field pose Pose Proposed local pose relative to the viewport.
---@field size SpatialSize Proposed hit-target dimensions in metres.

---@class SpatialSurface [proposed]
---@field id string Proposed stable spatial surface ID.
---@field surface_id string Proposed underlying Wayland surface ID.
---@field kind "window"|"layer" Proposed source surface kind.
---@field output_id string Proposed XR output ID.
---@field parent_id? string Proposed viewport or anchor parent; omitted for a root surface.
---@field pose Pose Proposed local pose when parented, otherwise pose in its space.
---@field space_id? string Proposed coordinate space for an unparented surface.
---@field size SpatialSize Proposed physical panel size in metres.
---@field curvature? SpatialCurvature Proposed optional horizontal curvature.
---@field focused boolean Proposed keyboard focus state.
---@field hovered boolean Proposed pointer hover state.
---@field stack_index integer Proposed spatial draw and hit-test stacking order.
---@field revision integer Proposed state revision.

---@class SpatialPointerEvent [proposed]
---@field source "controller"|"hand" Proposed tracked input source.
---@field phase "enter"|"leave"|"motion"|"press"|"release"|"cancel" Proposed pointer phase.
---@field target_id? string Proposed hit object ID.
---@field surface_id? string Proposed hit Wayland surface ID.
---@field x? number Proposed surface-local horizontal coordinate.
---@field y? number Proposed surface-local vertical coordinate.
---@field button? string Proposed logical trigger or hand-pinch button.
---@field tracking_valid boolean Proposed source tracking validity.
---@field time number Proposed monotonic event time in seconds.
---@field context FocusContext Proposed opaque, callback-scoped authority for related operations.

---@class SpatialGestureEvent [proposed]
---@field gesture "move"|"resize_corner" Proposed gesture kind.
---@field phase "begin"|"update"|"end"|"cancel" Proposed gesture phase.
---@field target_id string Proposed viewport or surface ID.
---@field corner? "north_east"|"north_west"|"south_east"|"south_west" Proposed resize corner.
---@field pose? Pose Proposed controller-derived pose, accepted only with context.
---@field context FocusContext Proposed opaque callback-scoped input authority.

---@class SpatialPlacement [proposed]
---@field key string Proposed application/role or user-selected persistence key.
---@field parent_id? string Proposed viewport or anchor ID.
---@field space_id? string Proposed space ID when placement is free.
---@field pose Pose Proposed pose in the selected parent or space.
---@field size SpatialSize Proposed physical panel size.
---@field curvature? SpatialCurvature Proposed optional surface curvature.

---@class SpatialAnimation [proposed]
---@field duration integer Proposed duration in milliseconds, from 0 to 10000.
---@field ease? string Proposed named easing curve.

---Return the output snapshot, including available XR outputs. [proposed]
---@return Output[] Proposed immutable output snapshots.
function gnoblin.outputs.list() end

---Find an output by stable ID. [proposed]
---@param id string Proposed stable output ID.
---@return Output|nil Proposed immutable output snapshot or nil.
function gnoblin.outputs.by_id(id) end

---Return spatial objects for an XR output. [proposed]
---@param output_id string Proposed XR output ID.
---@return {spaces: Space[], anchors: Anchor[], viewports: Viewport[], pills: Pill[], surfaces: SpatialSurface[]} Proposed immutable snapshot.
function gnoblin.spatial.snapshot(output_id) end

---Create a viewport on the selected XR output. [proposed]
---@param args {output_id: string, anchor_id: string, pose: Pose, size: SpatialSize, curvature?: SpatialCurvature} Proposed arguments.
---@return Operation<Viewport> Proposed compositor operation.
function gnoblin.spatial.create_viewport(args) end

---Create a pill hit target and associate its client artwork with a viewport. [proposed]
---@param args {viewport_id: string, surface_id: string, pose: Pose, size: SpatialSize} Proposed arguments.
---@return Operation<Pill> Proposed compositor operation.
function gnoblin.spatial.create_pill(args) end

---Set an object's transform, size, curvature, or parent. [proposed]
---@param id string Proposed viewport, pill, anchor, or spatial surface ID.
---@param args {pose?: Pose, size?: SpatialSize, curvature?: SpatialCurvature, parent_id?: string, space_id?: string, context?: FocusContext, animation?: SpatialAnimation} Proposed update fields.
---@return Operation<SpatialSurface|Viewport|Pill|Anchor> Proposed compositor operation.
function gnoblin.spatial.update(id, args) end

---Dock a window or layer surface to a viewport using a local pose. [proposed]
---@param surface_id string Proposed existing Gnoblin window or layer-surface ID.
---@param viewport_id string Proposed viewport ID.
---@param args {pose: Pose, size: SpatialSize, curvature?: SpatialCurvature, context?: FocusContext} Proposed local placement.
---@return Operation<SpatialSurface> Proposed compositor operation.
function gnoblin.spatial.dock(surface_id, viewport_id, args) end

---Place a window or layer surface freely under an anchor or space. [proposed]
---@param surface_id string Proposed existing Gnoblin window or layer-surface ID.
---@param args {anchor_id?: string, space_id?: string, pose: Pose, size: SpatialSize, curvature?: SpatialCurvature, context?: FocusContext} Proposed free placement.
---@return Operation<SpatialSurface> Proposed compositor operation.
function gnoblin.spatial.place(surface_id, args) end

---Change spatial keyboard focus using a live input context. [proposed]
---@param surface_id string Proposed target spatial surface ID.
---@param context FocusContext Proposed live pointer or gesture context.
---@return Operation<boolean> Proposed compositor operation.
function gnoblin.spatial.focus(surface_id, context) end

---Set a surface's spatial stacking index. [proposed]
---@param surface_id string Proposed target surface ID.
---@param index integer Proposed nonnegative stacking index.
---@return Operation<SpatialSurface> Proposed compositor operation.
function gnoblin.spatial.set_stack_index(surface_id, index) end

---Begin a compositor-managed corner resize from a live pointer context. [proposed]
---@param surface_id string Proposed spatial surface ID.
---@param corner "north_east"|"north_west"|"south_east"|"south_west" Proposed dragged corner.
---@param context FocusContext Proposed live pointer context.
---@return Operation<boolean> Proposed compositor operation.
function gnoblin.spatial.begin_resize(surface_id, corner, context) end

---Persist selected spatial placements using stable keys. [proposed]
---@param placements SpatialPlacement[] Proposed bounded, validated placements.
---@return Operation<{saved: integer}> Proposed compositor operation.
function gnoblin.spatial.save(placements) end

---Read persisted spatial placements. [proposed]
---@return SpatialPlacement[] Proposed validated saved records.
function gnoblin.spatial.restore() end

---Subscribe to a typed spatial pointer or gesture event. [proposed]
---@param name "gnoblin.spatial.pointer"|"gnoblin.spatial.gesture" Proposed event name.
---@param callback fun(event: SpatialPointerEvent|SpatialGestureEvent) Proposed non-blocking policy callback.
---@return Subscription Proposed unsubscribe handle.
function gnoblin.spatial.on(name, callback) end

---Subscribe to bounded scene-policy ticks; disabled unless registered. [proposed]
---@param callback fun(frame: {time: number, delta: number, output_id: string, revision: integer}) Proposed callback, capped at a proposed maximum of 30 Hz.
---@return Subscription Proposed unsubscribe handle.
function gnoblin.spatial.on_frame(callback) end
```

The sketch omits a generic transform setter on `Window`: the 2D window API remains valid for ordinary outputs, while the spatial API maps a stable window ID to a spatial surface. Typed methods can later move onto shared records if that proves clearer without changing units or authority rules.

## Complete policy examples

The examples use the proposed names above. They assume an XR output and its world anchor exist, and that layer surfaces are supplied by an ordinary Wayland client. IDs come from Gnoblin records; scripts do not invent surface IDs.

### Draggable pill moves its viewport

The compositor handles drag capture. Lua applies the tracked pose during the callback; C coalesces transform updates at a frame boundary.

```lua
local output = gnoblin.outputs.by_id("XR-0")
assert(output and output.kind == "xr")
local scene = gnoblin.spatial.snapshot(output.id)
local viewport = assert(scene.viewports[1])
local pill
for _, candidate in ipairs(scene.pills) do
    if candidate.viewport_id == viewport.id then pill = candidate; break end
end
assert(pill)

gnoblin.spatial.on("gnoblin.spatial.gesture", function(event)
    if event.gesture == "move" and event.target_id == pill.id
        and event.phase == "update" and event.pose then
        gnoblin.spatial.update(viewport.id, {
            pose = event.pose,
            context = event.context,
        })
    end
end)
```

### Dock a window to the viewport

Docking stores a local pose. Later viewport movement carries the child without a Lua callback or an operation on each frame.

```lua
local window = assert(gnoblin.windows.by_id("window-id-from-created-event"))
gnoblin.spatial.dock(window.id, viewport.id, {
    pose = {
        position = {x = 0.0, y = 0.0, z = -0.65},
        rotation = {x = 0.0, y = 0.0, z = 0.0, w = 1.0},
    },
    size = {width = 0.90, height = 0.56},
})
```

### Place a window in free space

This pose is relative to the world anchor and does not follow the viewport.

```lua
local window = assert(gnoblin.windows.by_id("window-id-from-created-event"))
gnoblin.spatial.place(window.id, {
    anchor_id = "world-origin",
    pose = {
        position = {x = 1.1, y = 0.2, z = -1.8},
        rotation = {x = 0.0, y = 0.12, z = 0.0, w = 0.993},
    },
    size = {width = 1.20, height = 0.75},
    curvature = {angle = 0.25},
})
```

### Resize by dragging a corner

Corner hit targets and geometry deltas are compositor work. Lua chooses which surfaces are resizable and authorizes an operation from the live gesture.

```lua
local resizable = {}
gnoblin.events.on("gnoblin.window.created", function(event)
    if event.window and event.window.resizable then
        resizable[event.window.id] = true
    end
end)

gnoblin.spatial.on("gnoblin.spatial.gesture", function(event)
    if event.gesture == "resize_corner" and event.phase == "begin"
        and resizable[event.target_id] and event.corner then
        gnoblin.spatial.begin_resize(
            event.target_id, event.corner, event.context)
    end
end)
```

The proposed resize continues natively until end or cancellation; Lua need not receive every pointer sample. A future API may expose resize progress for policies that need it.

### Move with menu button plus controller motion

The native recognizer emits `move` only while the configured menu button is held. It captures the initial controller-to-target relation to avoid a jump.

```lua
gnoblin.spatial.on("gnoblin.spatial.gesture", function(event)
    if event.gesture ~= "move" or event.phase ~= "update" or not event.pose then
        return
    end
    gnoblin.spatial.update(event.target_id, {
        pose = event.pose,
        context = event.context,
    })
end)
```

Production policy should retain movable IDs from window lifecycle events, rather than query a scene for every gesture update. The compositor still bounds and coalesces transform operations.

## Decisions required before implementation

1. Select the XR runtime and ownership boundary: in-process OpenXR,
   compositor-owned service, or another supported bridge. Verify lifecycle, device authorization, and output hotplug on the target headset.
2. Define XR output registration beside Mutter monitor outputs, including
   stable IDs, per-eye mode reporting, renderer sharing, suspend, mirroring, and runtime restart behavior.
3. Decide whether XR pointer delivery can reuse seat routing end to end or
   needs a narrowly scoped Gnoblin protocol addition. Do not publish a protocol name before it is designed and implemented.
4. Specify pose bounds, anchor relocalization, tracking-loss behavior, lock
   and privacy behavior, modal/transient placement, and multi-output focus.
5. Profile callback cadence, event queue limits, operation coalescing,
   animation cost, texture transport, memory bandwidth, and per-eye rendering on the actual host and headset. The 30 Hz tick is a starting proposal, not a measured performance claim.
6. Choose state schema, retention, reset controls, migration, and policy for
   multiple windows sharing an application ID.

## Repository sources

- `design/outputs.md` in `/var/tmp/gnoblin-outputs`: output record and reserved
  XR kind.
- `design/lua-api.md`: Lua operation, event, snapshot, security, and callback
  conventions.
- `design/lua-input-policy.md`: bounded input queues, event authority, lock
  isolation, and timeout/failure behavior.
- `design/monolithic-compositor.md`: compositor and supervised runtime
  ownership direction.
- `src/config/lua/api/monitors-layers.h`, `windows.h`, `input.h`,
  `animations.h`, and `events.h`: current LuaCATS style and adjacent APIs.
- `docs/shell-api/windows.md`, `layer-surfaces.md`, `input-handlers.md`,
  `appearance-animations.md`, and `docs/events/`: existing client and event contracts.
