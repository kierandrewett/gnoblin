# Wayland protocols for XR and spatial UI

**Status:** research and proposed design, checked 14 October 2026. This is not an implementation claim. Protocol maturity below reflects the upstream `wayland-protocols` tree and proposal state checked for this document; it can change.

## Finding

There is no Wayland protocol in the current upstream `wayland-protocols` tree that describes an XR session, headset view, spatial scene, 3D surface pose, reference space, or 6-DoF controller ray. Wayland has useful pieces for leasing a DRM connector, creating ordinary desktop surfaces, transporting buffers, synchronizing rendering, and capturing images. Those pieces do not define an XR desktop.

The closest upstream building blocks are `wp_drm_lease_device_v1` for direct display ownership and `xdg-shell`, layer-shell, and capture protocols for ordinary 2D surfaces. They leave scene composition and spatial input to another system. The closest additional proposal is `ext-toplevel-placement`, an open draft for trusted clients to suggest an initial 2D position on an `xdg_output`; it is a hint and does not represent a 3D pose.

For Gnoblin, retain ordinary Wayland surface roles and buffer rules. Define headset outputs, spatial placement, controller rays, and the mapping from spatial interaction to authorized Wayland input in Gnoblin's compositor and Lua control contract. A private Wayland protocol is only needed if an external headset renderer must exchange these records with Gnoblin.

Lua alone is adequate when the renderer is part of the supervised session or can use Gnoblin's authenticated control API.

## Relevant protocols and their limits

| Protocol                                                          | Upstream state                             | Useful for                                                              | Does not provide                                                              |
| ----------------------------------------------------------------- | ------------------------------------------ | ----------------------------------------------------------------------- | ----------------------------------------------------------------------------- |
| `wp_drm_lease_device_v1`                                          | Staging in `wayland-protocols`             | Leasing DRM connectors and related resources to a direct-display client | OpenXR session creation, compositor-managed XR rendering, scene or input APIs |
| `xdg-shell`                                                       | Stable                                     | Normal application toplevels, configure/acknowledge lifecycle, popups   | Client-selected absolute placement or 3D transforms                           |
| `zwlr_layer_shell_v1`                                             | External, unstable wlroots protocol        | Desktop panels and overlays attached to a 2D output                     | Portable standardized layer shell or spatial anchors                          |
| `ext-layer-shell-v1`                                              | Draft MR !28, not in current upstream tree | Proposed portable successor to the common layer-shell model             | A merged or broadly implemented standard; XR semantics                        |
| `ext-foreign-toplevel-list-v1`                                    | Staging                                    | Read-only enumeration of toplevels                                      | Window control, placement, capture, or scene management                       |
| `zwlr_foreign_toplevel_management_v1`                             | External, unstable wlroots protocol        | Toplevel metadata and selected management requests                      | A standardized 3D window manager                                              |
| `wp_viewport`                                                     | Stable                                     | Crop and scale a surface's 2D buffer content                            | Surface pose, perspective, stereoscopy, or head/world locking                 |
| `wp_fractional_scale_v1`                                          | Staging                                    | Advertise preferred fractional buffer scale                             | Physical angular resolution or headset lens/display calibration               |
| `wp_presentation`                                                 | Stable                                     | Report when a surface update is presented and timing metadata           | XR frame prediction, reprojection, or motion-to-photon guarantees             |
| `linux-dmabuf`                                                    | Stable                                     | Share GPU buffers and negotiate formats/modifiers                       | Synchronization by itself or a 3D scene protocol                              |
| `zwp_linux_explicit_synchronization_v1`                           | Unstable                                   | Explicit acquire/release fences for surface buffers                     | XR frame scheduling or display lease management                               |
| `wp_linux_drm_syncobj`                                            | Staging                                    | DRM syncobj timeline synchronization                                    | A replacement for scene or input protocols                                    |
| `ext_image_capture_source_v1` + `ext_image_copy_capture_v1`       | Staging                                    | Describe capture sources and request image copies                       | Unrestricted capture, low-latency continuous streaming, or XR presentation    |
| `zwlr_screencopy_manager_v1`                                      | External, unstable wlroots protocol        | Copy output regions or frames                                           | Universal per-window or layer-surface capture, scene data, or policy grants   |
| `wl_pointer`, tablet, relative-pointer and cursor-shape protocols | Core/stable/staging, depending on protocol | 2D pointer and stylus-like interaction                                  | A generic tracked 6-DoF controller pose or ray                                |

Staging means a protocol is ready for wider implementation but remains versioned for compatibility changes; experimental protocols can change more freely. The external `wlr-*` protocols are maintained outside this phase system. Do not treat an unstable protocol used by one compositor as a Wayland-wide guarantee.

### DRM leasing and an HMD connector

`wp_drm_lease_device_v1` is the protocol to inspect when an HMD is connected as a DRM/KMS display and a separate XR runtime needs direct scanout. The compositor advertises lease devices and connectors currently available for lease. A client creates a lease request, adds connector objects, submits it, and receives a file descriptor for the resulting DRM lease.

The XR runtime then programs the leased KMS resources through DRM; the Wayland protocol does not start OpenXR or deliver rendered eye images to the compositor.

The compositor must stop using leased resources for its own display modeset and scanout while the lease is active. It only offers connectors that can be safely handed off, after accounting for shared CRTCs, planes, and other DRM resources. A connector that becomes unavailable can be withdrawn before a lease is granted.

A granted lease can later be revoked: the client receives the lease's `finished` event and must stop using that lease. The client closes and releases its objects and DRM fd as required; the compositor regains the resources when the lease ends.

A device object's `release` request and `released` event destroy that client's enumeration object; they are not the event pair that grants a connector.

This is resource ownership transfer, not a general “make this monitor an XR output” request. Gnoblin should model it as an output lifecycle transition: available, offered/leased, withdrawn or revoked, and returned.

Keep the XR runtime's session state separate from the DRM lease state; an HMD can be streamed over USB/Wi-Fi or composed through a runtime without exposing a host DRM connector at all. Never claim lease success until the KMS lease fd and runtime display path are established.

Primary source: [DRM lease XML](https://gitlab.freedesktop.org/wayland/wayland-protocols/-/blob/main/staging/drm-lease/drm-lease-v1.xml), [staging directory](https://gitlab.freedesktop.org/wayland/wayland-protocols/-/tree/main/staging/drm-lease).

### Toplevels and layer surfaces

`xdg-shell` remains the interoperable role for application windows. The compositor sends size/state configures and the client acknowledges and commits buffers. The compositor owns final placement. `ext-foreign-toplevel-list` is a read-only list interface; it is useful for shells to discover windows, not to move or spatially place them.

`wlr-foreign-toplevel-management` is an external unstable protocol with management requests such as activate, close, maximize, minimize, and fullscreen. Its control surface still has no 3D pose. Gnoblin should keep trusted window-management policy in its Lua control API and avoid treating foreign-toplevel protocols as a spatial scene API.

`zwlr_layer_shell_v1` assigns a `wl_surface` to background, bottom, top, or overlay layers, with anchors, margins, desired size, keyboard interactivity, and an exclusive zone. Anchors constrain a rectangle against the edges of a 2D output. An exclusive zone reserves a strip of that output for other layer surfaces.

These concepts transfer naturally to a head-locked rectangular viewport: edge anchors and margins lay out content within the viewport, and an exclusive zone can reserve 2D viewport area for a toolbar or dock. They do not mean “attach to the headset” by themselves.

An overlay that follows head pose is head-locked. A panel attached to a controller, room surface, or stable point in the environment is world-locked. Neither coordinate system is represented in layer-shell XML. An exclusive zone should be interpreted only inside a declared 2D viewport; it must not silently reserve room-scale space or move other world-locked panels.

The proposed `ext-layer-shell-v1` has an open [draft MR !28](https://gitlab.freedesktop.org/wayland/wayland-protocols/-/merge_requests/28). The MR describes itself as an early copy of `wlr-layer-shell` with naming and API cleanup still under discussion. It was not present under `staging/` in the upstream tree checked on 14 October 2026. Even if merged unchanged, it would standardize 2D desktop layers, not XR anchoring.

Another open [draft, `ext-toplevel-placement-v1` MR !389](https://gitlab.freedesktop.org/wayland/wayland-protocols/-/merge_requests/389), proposes trusted clients suggesting an initial logical position relative to an `xdg_output`. The compositor may restrict clients, ignore the suggestion, or choose another position. It is useful precedent for policy-gated placement hints, but it is 2D and does not solve persistence or spatial poses.

Primary sources: [xdg-shell XML](https://gitlab.freedesktop.org/wayland/wayland-protocols/-/blob/main/stable/xdg-shell/xdg-shell.xml), [wlr layer-shell XML](https://github.com/swaywm/wlr-protocols/blob/master/unstable/wlr-layer-shell-unstable-v1.xml), [ext foreign toplevel list XML](https://gitlab.freedesktop.org/wayland/wayland-protocols/-/blob/main/staging/ext-foreign-toplevel-list/ext-foreign-toplevel-list-v1.xml), [wlr foreign-toplevel management XML](https://github.com/swaywm/wlr-protocols/blob/master/unstable/wlr-foreign-toplevel-management-unstable-v1.xml).

### Scale, presentation, buffers, and capture

`wp_viewport` sets a source rectangle and destination size for a surface's buffer. It is useful for fitting a captured application image into a panel. `wp_fractional_scale_v1` lets a client learn the preferred fractional scale for producing buffers. Both operate on 2D surface content.

A headset renderer must separately choose panel pixel density, physical size, eye projection, and any resampling between the application's buffer and XR swapchain.

`wp_presentation` reports presentation feedback for a surface commit, with a timestamp and refresh sequence where available. This helps measure a 2D surface's presentation. It does not promise predicted XR display time, late-latching, asynchronous reprojection, or headset frame pacing.

`linux-dmabuf` negotiates shareable buffer formats and modifiers. Explicit synchronization protocols communicate when a buffer may be read and when its use has finished. Correct use can avoid unnecessary copies, but neither a DMA-BUF handle nor a successful import proves a zero-copy path.

Gnoblin's renderer and headset runtime must negotiate compatible formats/modifiers, bound in-flight buffers, wait on fences, and handle fallback copies. The upstream tree also has staging `linux-drm-syncobj`; do not confuse it with the older external unstable `zwp_linux_explicit_synchronization_v1`.

For capture, `ext_image_capture_source_v1` identifies an authorized source; `ext_image_copy_capture_v1` requests copied frames from such a source. They are staging protocols and still require a compositor to expose the relevant source and enforce its capture policy. The source protocol does not turn all surfaces into capturable images. `zwlr_screencopy_manager_v1` is a separate wlroots protocol for output capture.

It does not provide a portable per-window stream, nor does it guarantee capture of layer surfaces. None of these protocols moves a window, exposes scene geometry, or grants input to the captured surface.

Primary sources: [viewporter](https://gitlab.freedesktop.org/wayland/wayland-protocols/-/blob/main/stable/viewporter/viewporter.xml), [fractional scale](https://gitlab.freedesktop.org/wayland/wayland-protocols/-/blob/main/staging/fractional-scale/fractional-scale-v1.xml), [presentation-time](https://gitlab.freedesktop.org/wayland/wayland-protocols/-/blob/main/stable/presentation-time/presentation-time.xml), [linux-dmabuf](https://gitlab.freedesktop.org/wayland/wayland-protocols/-/blob/main/stable/linux-dmabuf/linux-dmabuf-v1.xml), [unstable explicit synchronization](https://gitlab.freedesktop.org/wayland/wayland-protocols/-/blob/main/unstable/linux-explicit-synchronization/linux-explicit-synchronization-unstable-v1.xml), [staging DRM syncobj](https://gitlab.freedesktop.org/wayland/wayland-protocols/-/blob/main/staging/linux-drm-syncobj/linux-drm-syncobj-v1.xml), [image capture source](https://gitlab.freedesktop.org/wayland/wayland-protocols/-/blob/main/staging/ext-image-capture-source/ext-image-capture-source-v1.xml), [image copy capture](https://gitlab.freedesktop.org/wayland/wayland-protocols/-/blob/main/staging/ext-image-copy-capture/ext-image-copy-capture-v1.xml), [wlr screencopy XML](https://github.com/swaywm/wlr-protocols/blob/master/unstable/wlr-screencopy-unstable-v1.xml).

## XR and spatial work outside the standard tree

The upstream `wayland-protocols` tree was checked for paths and proposals named for XR, spatial scenes, 3D surfaces, and layer shell. It contains no XR or 3D-surface protocol. Its experimental `xx-*` directory is not an XR namespace; current entries include unrelated experimental protocols. The `ext-layer-shell` proposal is tracked as MR !28 rather than a merged protocol.

Collabora contributes to Wayland protocol work, including DRM leasing, DMA-BUF, and the viewporter. I could not verify a public, authoritative Collabora protocol repository or specification named “wxrc” during this research. Do not cite “wxrc” as an existing protocol without a repository, XML file, or proposal URL. The similarly named `wayland-protocols` work from Collabora does not itself establish an XR protocol.

Stardust XR is a separate XR environment and runtime project, with its own client/server components and Gluon protocol definitions for spatial and tracked objects. It is useful architectural prior art for a scene graph and tracked spatial devices, but Gluon is not a Wayland protocol and does not make arbitrary Wayland compositors compatible.

Likewise, KDE, Hyprland/wlroots, Smithay, and Sway may implement or experiment with particular Wayland protocols; their compositor APIs and private extensions are not automatically portable between them. This research found no upstream XR/spatial Wayland protocol proposal from those projects that would supply the missing headset scene and 6-DoF input contract.

Sources: [wayland-protocols protocol phases and tree](https://gitlab.freedesktop.org/wayland/wayland-protocols/-/blob/main/README.md), [Collabora's public project list](https://gitlab.freedesktop.org/Collabora), [Stardust XR organization](https://github.com/StardustXR), [Stardust XR core and Gluon protocol sources](https://github.com/StardustXR/core/tree/main/protocol), [Stardust XR server](https://github.com/StardustXR/server).

## Controller input

`wl_pointer` is a seat's 2D pointer: enter/leave, surface-local motion, buttons, and scroll. The client may provide a cursor surface and hotspot. `zwp_relative_pointer_v1` reports relative motion and is useful for locked desktop-pointer modes, not for a tracked pose. The tablet protocol models a stylus with proximity, tip, buttons, pressure, distance, tilt, and rotation.

It has no canonical 6-DoF pose, ray, controller identity model, or target surface-local mapping. `wp_cursor_shape_device_v1` selects a standard cursor shape; it says nothing about how a controller ray is aimed.

A headset application can raycast locally against its scene and move a normal Wayland pointer to the hit point, but then the renderer is responsible for keeping the ray, hovered image, and click coordinate in sync. To inject input from a separate privileged client, `zwlr_virtual_pointer_v1` is an external unstable option, subject to compositor support and authorization.

It emits ordinary 2D pointer events; it cannot carry a 3D pose. Mapping the controller as a tablet tool is also lossy and can misrepresent pointer identity and semantics.

For Gnoblin, process controller pose in the trusted XR input path. Lua chooses the interaction policy and gesture; Gnoblin validates the current target and converts an authorized ray hit into normal surface-local pointer events. Keep the ray, hover endpoint, click, and resize/move gesture tied to one mapping revision. On tracking loss, cancel held buttons and active gestures.

Never let a client invent input serials or use controller coordinates to bypass focus, lock, or input-capture policy.

Primary sources: [Wayland core XML](https://gitlab.freedesktop.org/wayland/wayland/-/blob/main/protocol/wayland.xml), [tablet-v2 XML](https://gitlab.freedesktop.org/wayland/wayland-protocols/-/blob/main/stable/tablet/tablet-v2.xml), [relative pointer XML](https://gitlab.freedesktop.org/wayland/wayland-protocols/-/blob/main/unstable/relative-pointer/relative-pointer-unstable-v1.xml), [cursor shape XML](https://gitlab.freedesktop.org/wayland/wayland-protocols/-/blob/main/staging/cursor-shape/cursor-shape-v1.xml), [wlr virtual pointer XML](https://github.com/swaywm/wlr-protocols/blob/master/unstable/wlr-virtual-pointer-unstable-v1.xml).

## What Gnoblin must define

The owner has reserved an `xr` output kind in the output design. Treat the headset as an output record in Gnoblin's output model, while keeping these states distinct:

- **XR session:** runtime/session availability, tracking state, reference
  spaces, and display timing.
- **Display route:** direct DRM lease, runtime-owned display, or streamed
  headset. A lease is one possible route, not the definition of an XR output.
- **XR viewport:** eye/view configuration and the rendered view presented to
  the headset.
- **Wayland surface:** an ordinary `xdg_toplevel` or layer surface and its
  2D buffer/configure lifecycle.
- **Spatial placement:** a compositor-known transform and anchor for showing
  that surface in the XR scene.
- **Input mapping:** a tracked controller ray, selected target, and authorized
  Wayland input sequence.

The scene policy belongs in Lua as requested: which layer surfaces dock to a head-locked viewport, which windows float in world space, how dragging and resize gestures work, and how controllers select them. Gnoblin remains the authority for output lifecycle, stable surface/window identity, permissions, focus, input injection, lock isolation, and cancellation when the runtime or tracking fails.

A shell or XR renderer owns drawing the scene; it should not be the authority that fabricates compositor input.

### Minimal Lua-level contract (proposal)

The following names and records are a design sketch, not existing API names. Prefer this route if the XR renderer is part of the supervised Gnoblin session or can call its authenticated control channel:

```lua
-- Read-only snapshot; IDs and field names here are proposed.
local output = gnoblin.outputs.get("xr:0")
local session = gnoblin.xr.session(output.id)

gnoblin.xr.on_surface_available(function(surface, context)
  if surface.role == "layer" then
    return {
      anchor = "head",       -- "head" or "world"
      viewport = "primary",  -- named XR viewport
      layout = "dock",
    }
  end
  return { anchor = "world", pose = context.suggested_pose }
end)

gnoblin.xr.on_controller(function(controller, hit, context)
  -- hit is validated for this frame/mapping revision by Gnoblin.
  if controller.buttons.trigger == "pressed" and hit then
    return { action = "pointer_press", target = hit.surface_id,
             mapping_revision = hit.mapping_revision }
  end
  return { action = "hover", target = hit and hit.surface_id }
end)
```

Required contract properties:

- Stable IDs for XR outputs, viewports, surfaces, and controllers; snapshots
  carry revisions so Lua can reject stale poses and targets.
- Explicit anchors and reference spaces, at minimum `head` and `world`;
  transforms include position, orientation, units, and the reference-space revision. Define whether a pose is a suggestion or accepted placement.
- A 2D viewport layout for layer-shell-derived docks. Anchors and exclusive
  zones apply only inside that viewport. World-locked surfaces use a 3D pose and do not consume a 2D exclusive zone.
- Controller pose/ray and button state arrive with a monotonic timestamp,
  tracking validity, device identity, and mapping revision. Lua chooses policy; the compositor validates hit-testing and target eligibility.
- Pointer actions resolve to surface-local coordinates at the same mapping
  revision used for hover. Press/release ownership, focus, session lock, grabs, and cancellation remain compositor-enforced.
- Async operations report success, denial, unsupported route, stale revision,
  tracking loss, device removal, runtime loss, and timeout. Session shutdown or lease revocation retires input and placement state safely.
- No Lua access to raw Mutter actor pointers, renderer-private objects,
  arbitrary DRM fds, or client-supplied input serials.

### When a private Wayland protocol is justified

If an independently developed headset renderer must receive surface images and placement state, expose a narrow private protocol or documented Lua/control API instead of extending layer-shell with unrelated scene concepts. A minimal protocol would need objects conceptually equivalent to:

1. `gnoblin_xr_manager`: enumerate XR outputs/viewports and report capabilities.
2. `gnoblin_xr_surface`: bind an authorized `wl_surface` or Gnoblin surface ID
   to a spatial presentation; report configure/accepted state and lifecycle.
3. `gnoblin_xr_controller`: deliver timestamped pose, ray, buttons, and
   tracking validity; accept only compositor-authorized input actions.
4. Events for output/session changes, surface placement revisions,
   presentation feedback, controller loss, and cancellation.

Do not specify message names or freeze XML until a real client boundary requires them. The hard design questions are reference-space ownership and relocalization, pose authority, per-client authorization, capture/content transport, frame timing, and how surface-local coordinates stay aligned with the rendered panel.

If Gnoblin and the renderer share a supervised session, typed Lua operations and events avoid creating a second public protocol and keep policy in the requested place.

## Sources and verification scope

Protocol phase and file-path checks used the upstream [wayland-protocols tree](https://gitlab.freedesktop.org/wayland/wayland-protocols/-/tree/main). Layer-shell and toplevel-placement proposal status came from [MR !28](https://gitlab.freedesktop.org/wayland/wayland-protocols/-/merge_requests/28) and [MR !389](https://gitlab.freedesktop.org/wayland/wayland-protocols/-/merge_requests/389). External protocols are linked to their maintainers' XML repositories above. This document is protocol research and a design sketch; it does not establish Gnoblin implementation, headset hardware support, capture performance, or a working DRM lease/runtime path.
