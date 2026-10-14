/* LuaCATS for the optional Gnoblin VR extension. These methods are installed
 * beneath gnoblin.spatial by the extension and use vr.spatial.* control routes. */

/*** lua
---@class GnoblinVRPose
---@field position {x:number, y:number, z:number} Translation in metres.
---@field rotation {x:number, y:number, z:number, w:number} Unit quaternion.

---@class GnoblinVRSize
---@field width number Visible panel width in metres.
---@field height number Visible panel height in metres.

---@class GnoblinVRSpatialSurface
---@field id string Stable Gnoblin window or layer surface ID.
---@field pose GnoblinVRPose Pose relative to its viewport or anchor.
---@field size GnoblinVRSize Physical panel size.
---@field anchor_id? string Free-space anchor ID.
---@field viewport_id? string Viewport ID when attached to the viewport pill.
---@field stack_index integer Spatial stacking rank used for rendering and hit testing.

---@class GnoblinVRVector
---@field x number
---@field y number
---@field z number

---@class GnoblinVRPointArgs
---@field origin GnoblinVRVector Ray origin in metres.
---@field direction GnoblinVRVector Ray direction; need not be normalised.
---@field button? "left"|"right"|"middle" Button to press or release; needs state.
---@field state? "press"|"release" What to do with button.

---@class GnoblinVRPointResult
---@field hit boolean Whether the ray crossed a surface.
---@field delivered boolean Whether the pointer was moved onto a window.
---@field surface_id? string Surface that was hit.
---@field u? number Horizontal position on it, 0 to 1.
---@field v? number Vertical position on it, 0 to 1.
---@field distance? number Ray distance in metres.

---@class GnoblinVRSpatialHit
---@field surface_id string Window or layer surface ID hit by the controller ray.
---@field x number Surface-local x coordinate in logical pixels.
---@field y number Surface-local y coordinate in logical pixels.
---@field distance number Ray distance in metres.

---Read the extension-owned XR scene snapshot.
---@return Operation<GnoblinVRSpatialSurface[]> Asynchronous scene snapshot.
function gnoblin.spatial.snapshot() end

---Create an extension-owned spatial anchor.
---@param args {id:string, pose:GnoblinVRPose, space_id?:string}
---@return Operation<{id:string, pose:GnoblinVRPose, space_id:string}>
function gnoblin.spatial.anchor(args) end

---Place a window or layer surface at a free-space anchor.
---@param args {surface_id:string, anchor_id:string, pose:GnoblinVRPose, size:GnoblinVRSize}
---@return Operation<GnoblinVRSpatialSurface>
function gnoblin.spatial.place(args) end

---Attach a window or layer surface to the viewport pill.
---@param args {surface_id:string, viewport_id:string, pose:GnoblinVRPose, size:GnoblinVRSize}
---@return Operation<GnoblinVRSpatialSurface>
function gnoblin.spatial.attach(args) end

---Move a surface using a hit record provided by an input policy.
---@param args {surface_id:string, hit:GnoblinVRSpatialHit, pose:GnoblinVRPose}
---@return Operation<GnoblinVRSpatialSurface>
function gnoblin.spatial.move(args) end

---Raise a surface from the back of the spatial stack.
---@param args {surface_id:string}
---@return Operation<GnoblinVRSpatialSurface>
function gnoblin.spatial.raise(args) end

---Cast a ray into the scene and report the nearest surface it crosses.
---Surfaces under a head-locked anchor are skipped. `u` and `v` run from 0 to 1 across the surface,
---left to right and top to bottom.
---@param args {origin:{x:number, y:number, z:number}, direction:{x:number, y:number, z:number}}
---@return Operation<{hit:boolean, surface_id?:string, u?:number, v?:number, distance?:number}>
function gnoblin.spatial.hit(args) end

---Point at a window with a controller ray. The pointer moves to where the ray crosses the window,
---which is raised; with `button` and `state` the button is pressed or released there, so the
---window receives focus, motion and clicks. The surface ID must be a Gnoblin window ID.
---@param args GnoblinVRPointArgs
---@return Operation<GnoblinVRPointResult>
function gnoblin.spatial.point(args) end

---Report where the head is in world space. Anchors created with `space_id = "view"` follow it, so
---`hit` can reach head-locked surfaces. The presentation client sends this each frame.
---@param args {pose:GnoblinVRPose}
---@return Operation<{revision:integer}>
function gnoblin.spatial.head(args) end

---Update a placed surface pose.
---@param args {surface_id:string, pose:GnoblinVRPose}
---@return Operation<GnoblinVRSpatialSurface>
function gnoblin.spatial.update_pose(args) end
*/
