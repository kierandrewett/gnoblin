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
2. Runtime bridge as an optional extension: an extension-owned supervised presentation client opens the OpenXR session (Monado first), obtains the headset, and exposes per-eye modes plus availability to Lua. Nothing is placed in the scene yet. Gnoblin core's output list remains derived from Mutter's monitor manager; the extension-to-driver record path for an XR output must be established before core can list it.
   2a. Preview window, no headset: the extension can run the presentation client in Monado's windowed mode (`XRT_COMPOSITOR_FORCE_XCB=1` or the Wayland equivalent), rendering the composition view into a normal desktop window. This makes the per-eye mode, availability and refresh rate observable without a headset, and it becomes the change-over-the-wire validation the headset later replaces. Software-rendered sessions add a constraint: the Monado GL path imports swapchain textures through `GL_EXT_memory_object_fd`, so a preview window stays dark on stacks whose GL does not expose that extension (for example llvmpipe in a virtual machine). The session, swapchains, and animated test pattern still run, and real OpenGL drivers with `GL_EXT_memory_object_fd` show the pattern.
3. Lua surface: the VR extension installs its `gnoblin.spatial` LuaCATS surface through the extension installer hook. Anchor, free-space placement, viewport attachment, move, raise, and pose-update calls validate their input and queue typed actions. The compositor action handler, OpenXR texture import and quad projection, 3D ray hit testing, pointer/focus delivery, and viewport pill gesture policy are not implemented in this slice. The compositor holds a validated scene store (anchors, placements, stacking, poses) that the Lua calls write to; it does not yet place a visible XR scene.
4. Interaction: controllers as pointers (rays with select, trigger, menu and grip poses), drag-resize from window corners, move by menu plus motion, and the dock-to-viewport layer-shell binding. Compositor events stay bounded and coalesced as in design/lua-input-policy.md.
5. Hand tracking, curvature, persistence, privacy behavior under session lock, and multi-headset output enumeration follow after the MVP proves the cost model on real hardware.

Phase 3 API validation runs without a headset. Visible placement and interaction still need the compositor action handler and projection/input integration, followed by validation in the Monado preview window and a qualified headset before any performance claim in phase 5.

## Links

- Outputs API design and implementation live in the outputs worktree until merged; `kind = "xr"` is reserved in design/outputs.md.
- Input policy constraints: design/lua-input-policy.md.
- Lua conventions and security: design/lua-api.md.
