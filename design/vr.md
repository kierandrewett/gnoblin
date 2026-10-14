# VR and spatial outputs

An xr headset appears in Gnoblin's outputs API, and the Lua runtime owns every decision about what appears in it and where. This page states the position, the plan to reach it, and the research behind each choice. It is a design position, not implemented behavior.

## Position

- The headset renders through an OpenXR runtime. Gnoblin does not become the headset's compositor. A presentation client receives authorised window frames and Lua-selected poses and submits composition layers.
- Lua owns the spatial scene: which surfaces are in it, their transforms, what docks to the viewport, what sits in free space, and gesture mapping. The C compositor owns the OpenXR session, frame timing, tracking data, surface import, projection, hit testing and failure recovery, and it must never call Lua on the render or tracking thread.
- There is no Wayland protocol for XR scenes today. DRM leasing and ordinary surface protocols are all upstream offers. Spatial placement, controller rays and the input mapping stay in Gnoblin's compositor and Lua contract; a private protocol is only needed if an external renderer must exchange them.
- The outputs API describes the headset as an output with a stable ID and per-eye modes, not as a logical desktop rectangle.

## Documents

| Document                                     | Content                                                                                  |
| -------------------------------------------- | ---------------------------------------------------------------------------------------- |
| [OpenXR runtimes](vr/openxr-runtimes.md)     | Runtime landscape, Monado's modes and service model, why the runtime stays out of Mutter |
| [Wayland protocols](vr/wayland-protocols.md) | What upstream offers, the layer-shell mapping, the proposed Lua contract                 |
| [Lua model](vr/lua-model.md)                 | Proposed spatial API surface with LuaCATS annotations, and worked examples               |
| [Lua control audit](lua-control.md)          | What the compositor and gnoblinctl do without Lua today, and the sequence of Lua PRs     |
| [GNOME parity](gnome-parity.md)              | Remaining GNOME-shell parity gaps and the matching backlog                               |

## Phase plan

1. Outputs surface, without a runtime: the outputs API reports an `xr` output when a leased headset connector is present, with a stable ID. `gnoblinctl output` prints it. Lua can enumerate it and see enable state, but nothing renders in it yet.
2. Runtime bridge behind the outputs API: a supervised presentation client opens the OpenXR session (Monado first), leases or otherwise obtains the headset, and reports per-eye modes plus availability into the outputs snapshot. Lua can enable, disable and see the runtime. Nothing is placed in the scene yet.
3. Scene MVP: import window textures as OpenXR quad composition layers, place them through `gnoblin.spatial` operations issued from Lua, with the compositor performing projection and hit testing. Focus follows the pointer ray in 3D. The viewport pill exists, and dragging it is a Lua gesture policy fed by controller input.
4. Interaction: controllers as pointers (rays with select, trigger, menu and grip poses), drag-resize from window corners, move by menu plus motion, and the dock-to-viewport layer-shell binding. Compositor events stay bounded and coalesced as in design/lua-input-policy.md.
5. Hand tracking, curvature, persistence, privacy behavior under session lock, and multi-headset output enumeration follow after the MVP proves the cost model on real hardware.

Phase 3 needs a qualified headset or a validated streaming runtime before any performance claim in phase 5.

## Links

- Outputs API design and implementation live in the outputs worktree until merged; `kind = "xr"` is reserved in design/outputs.md.
- Input policy constraints: design/lua-input-policy.md.
- Lua conventions and security: design/lua-api.md.
  EOF echo written
