# OpenXR runtimes and the Wayland desktop

**Status:** research and architecture recommendation for Gnoblin. Runtime features and project status can change; extension availability must be queried at runtime. This document describes an integration direction, not implemented Gnoblin behavior.

## Recommendation

Keep the OpenXR runtime responsible for headset tracking, frame timing, reprojection, and final display composition. Keep Gnoblin responsible for Wayland surfaces, focus, input authorization, outputs, and Lua policy. Add a small OpenXR presentation client that receives authorized window frames and Lua-selected poses, then submits one projection view plus window composition layers.

Lua remains the decision maker; the client is a renderer and input bridge.

Do not make Mutter the XR runtime or the headset’s final compositor. Replacing Mutter’s 2D scene renderer with a per-eye XR compositor would couple Gnoblin to device drivers, OpenXR session lifecycle, runtime-specific swapchains, reprojection deadlines, and direct-display ownership. Existing OpenXR runtimes already implement those responsibilities.

A secondary XR-only Wayland compositor is useful for a separate VR login/session, but it does not naturally bring the current Gnoblin windows with it.

## Runtime landscape

An OpenXR application links to the Khronos loader. The loader selects one runtime manifest, normally via `XR_RUNTIME_JSON` or the active runtime JSON. The runtime supplies the OpenXR implementation, device drivers, compositor and tracking/action data. An application renders into runtime-owned swapchains and calls `xrEndFrame`; the runtime handles final headset presentation and any runtime-specific display path.

This makes the runtime an output backend for an application, not a Wayland output that Gnoblin can simply enumerate as a monitor. [OpenXR loader and runtime selection](https://github.com/KhronosGroup/OpenXR-SDK-Source/blob/main/README.md), [Monado runtime overview](https://monado.freedesktop.org/about-runtimes.html)

### [Monado](https://gitlab.freedesktop.org/monado/monado)

- **Linux role and service:** Open-source OpenXR runtime with headset/controller drivers, compositor and optional `monado-service`. The normal service build runs drivers and compositor out of process over local IPC (`$XDG_RUNTIME_DIR/monado_comp_ipc`); systemd socket activation is supported. A no-service build loads the runtime in the app and cannot provide `XR_EXTX_overlay` multi-app sessions.
- **Display and graphics path:** Direct mode on Linux uses GPU/DRM access or Wayland DRM leasing where supported.
    - It can fall back to extended-display mode.
    - It can also run the compositor through Wayland or XCB; `XRT_COMPOSITOR_FORCE_XCB=1` explicitly selects a windowed compositor and `XRT_COMPOSITOR_XCB_FULLSCREEN=1` makes that window fullscreen.
    - Wayland direct mode can request a lease through `wp_drm_lease_v1`.
- **What it means for Gnoblin:** Best open implementation target and easiest to inspect. Do not assume every Monado driver exposes every input profile or extension.

### [WiVRn](https://github.com/WiVRn/WiVRn)

- **Linux role and service:** Streaming system for standalone Android headsets.
    - The headset connects to `wivrn-server`; when connected, the server selects WiVRn’s OpenXR/OpenVR configuration, and restores it after applications exit and the headset disconnects.
    - WiVRn can launch overlay clients such as WayVR.
- **Display and graphics path:** Host renders with Vulkan and encodes frames for network delivery; headset-side client decodes and presents them. It avoids a physical HMD connector and DRM lease on the host. Host encoder availability, Wi-Fi and encode/decode latency are part of the display path.
- **What it means for Gnoblin:** WiVRn provides its own OpenXR runtime for the streaming path; it is a separate runtime choice, not a Monado transport driver.
    - The headset connection gates runtime availability.
    - Keep streaming/encoding out of Mutter and treat WiVRn as an interchangeable runtime, not a Gnoblin subsystem.

### [ALVR](https://github.com/alvr-org/ALVR)

- **Linux role and service:** Host streamer consists of a dashboard plus a driver loaded by SteamVR; Android headset client is an OpenXR application. ALVR is not itself the normal Linux host OpenXR runtime.
- **Display and graphics path:** On Linux ALVR’s SteamVR driver obtains rendered frames through a Vulkan WSI layer because the OpenVR direct-mode driver path used on Windows is unavailable.
    - Host compositor capture, encoding (VAAPI, NVENC, AMF or software x264), network transport and headset decode add cost and failure points.
- **What it means for Gnoblin:** Useful as a streaming path when SteamVR is already the chosen runtime. Avoid making Gnoblin depend on ALVR-specific capture or frame hooks. Its OpenXR headset client does not imply the Linux host application should use ALVR’s client runtime.

### [SteamVR for Linux](https://github.com/ValveSoftware/SteamVR-for-Linux)

- **Linux role and service:** Proprietary runtime and compositor, providing OpenVR and OpenXR. Linux users commonly run the SteamVR runtime and drivers through Steam.
- **Display and graphics path:** Vulkan-based; wired HMD paths need a DRM lease from the Wayland compositor or an X11 setup.
    - Remote headset support may instead arrive through ALVR’s SteamVR driver.
    - Valve’s current support page still describes compositor DRM leasing as a requirement; test the actual SteamVR release, GPU and compositor because support is version- and driver-sensitive.
- **What it means for Gnoblin:** Valuable compatibility target but not the portability baseline. Test `wp_drm_lease_v1`, leased connector visibility, GPU choice and Steam runtime/container access separately.

### Monado details that affect integration

Monado has multiple ways to reach a display. In direct mode it bypasses normal window-system presentation so the runtime can own headset scanout and synchronization. Linux GPU-specific paths include NVIDIA’s Vulkan display route and Mesa/KMS routes; on Wayland, the runtime can request a DRM lease.

When direct mode is unavailable, extended mode treats the headset as a regular monitor and the runtime presents its corrected image there. For development and fallback, the compositor can be nested in an XCB window or run on Wayland. A nested window is not equivalent to direct HMD scanout and adds the host compositor to the presentation chain.

[Monado direct-mode guide](https://monado.freedesktop.org/direct-mode.html), [Monado compositor settings](https://monado.freedesktop.org/getting-started.html#environment-variables)

With the normal service configuration, the service owns the device drivers and compositor; OpenXR clients connect through Monado’s IPC. This separation is important for overlays: a second presentation process needs a persistent service and the runtime’s multi-application extension support. Running the runtime in-process per app is simpler but prevents independent overlay sessions. [Monado service documentation](https://monado.freedesktop.org/getting-started.html#monado-service)

The Monado compositor requires Vulkan external-memory and external-semaphore FD support for application swapchain images, including `VK_KHR_external_memory_fd` and `VK_KHR_external_semaphore_fd`. Monado reports reasonably modern Mesa RADV, Intel ANV and proprietary NVIDIA Vulkan drivers as tested. OpenGL applications have a separate requirement: `GL_EXT_memory_object_fd`; Mesa Intel has format limitations.

This is a runtime GPU/driver requirement, not a requirement to use the same GPU for Gnoblin’s ordinary 2D outputs. Multi-GPU arrangements can fail or require explicit GPU selection, and same-GPU app/runtime placement is the safer first target. [Monado graphics requirements](https://monado.freedesktop.org/getting-started.html#monado-requirements), [multi-GPU notes](https://monado.freedesktop.org/multi-gpu.html)

For interaction, OpenXR standardizes action sets, actions, action spaces and suggested bindings; it does not standardize one universal button layout or controller model. Applications should bind `aim` and `grip` poses plus `select`, trigger and menu actions through available interaction profiles.

The core Khronos Simple Controller profile provides `select`, `menu`, `grip/pose`, `aim/pose` and haptics; richer profiles such as HTC Vive, Valve Index or Meta Touch add vendor/device-specific components and still need runtime bindings.

Treat `isActive` and pose validity as state, and cancel drags when tracking is lost.

`XR_EXT_hand_tracking` exposes joint locations when supported; `XR_EXT_hand_interaction` adds standard hand pinch/poke pose paths to interaction profiles.

Availability depends on device, driver and runtime. Controller emulation from hands can help controller-only applications, but a compositor should not assume it is present. [OpenXR input specification](https://registry.khronos.org/OpenXR/specs/1.1/html/xrspec.html#input), [`XR_EXT_hand_interaction`](https://registry.khronos.org/OpenXR/specs/1.1/man/html/XR_EXT_hand_interaction.html), [Monado hand tracking](https://monado.freedesktop.org/handtracking)

### Extensions worth checking

Call `xrEnumerateInstanceExtensionProperties` and query system properties for the selected runtime/device. Do not infer support from a headset brand or from support in another runtime. These are the extensions most relevant to a desktop presentation client:

### `XR_KHR_vulkan_enable2` (or `XR_KHR_vulkan_enable`)

- **Use and limits:** Vulkan graphics binding and runtime-selected Vulkan device/requirements. The exact variant and version differ by runtime. Use the runtime’s recommended physical device for swapchain sharing; do not presume Mutter’s device is compatible.

### `XR_EXT_hand_tracking`

- **Use and limits:** Optional articulated hand joints. Useful for pinch/gesture input when available; it does not replace Gnoblin’s authorized pointer/key delivery path.

### `XR_EXT_hand_interaction`

- **Use and limits:** Adds standardized `/input/pinch_ext/pose` and `/input/poke_ext/pose` paths to hand-capable interaction profiles. Useful for optional hand pointing and pinch gestures; check support alongside hand tracking.

### `XR_EXTX_overlay`

- **Use and limits:** Experimental overlay-session extension for a separate OpenXR process to submit content over a primary application. Runtime support is not universal. Monado documents this in its service mode; query before depending on it.

### `XR_KHR_composition_layer_quad`

- **Use and limits:** **Not an extension name:** `XrCompositionLayerQuad` is core OpenXR. It places one textured rectangle in an `XrSpace`; it is the basic panel/window primitive.

### `XR_KHR_composition_layer_cylinder`, `XR_KHR_composition_layer_equirect2`, `XR_KHR_composition_layer_depth`

- **Use and limits:** Optional curved panels, panoramic layers and depth-aware projection-layer composition. Check per runtime. They are enhancements; a quad path should remain the portable baseline.

### `XR_MND_headless`

- **Use and limits:** Monado-specific session without graphical output. Useful for tracking/action tooling and headless tests, not for presenting the desktop.

### `XR_MND_swapchain_usage_input_attachment_bit`

- **Use and limits:** Monado extension for requesting input-attachment swapchain usage. Not required for ordinary window panels.

### `XR_MNDX_egl_enable`

- **Use and limits:** Monado’s EGL graphics binding option. Prefer Vulkan for a first shared-buffer path because Monado’s published Vulkan external-memory requirements are explicit.

Monado publishes support for the KHR depth/cylinder/equirect layers, `XR_EXT_hand_tracking`, `XR_EXTX_overlay`, `XR_MND_headless`, and the MND swapchain/EGL extensions. Khronos’ runtime extension inventory is the better way to compare runtime declarations, but a runtime advertising an extension does not prove that a particular device/driver path works. [Monado extension list](https://monado.freedesktop.org/), [Khronos runtime extension inventory](https://github.khronos.org/OpenXR-Inventory/runtime_extension_support.html), [OpenXR quad reference](https://registry.khronos.org/OpenXR/specs/1.1/man/html/XrCompositionLayerQuad.html)

## How an OpenXR application renders

A normal OpenXR application creates an instance and session, selects a graphics binding, obtains per-view recommendations, creates swapchains, waits/begins each frame, locates views and spaces at the predicted display time, renders, then passes layer headers to `xrEndFrame`. The usual scene is an `XrCompositionLayerProjection` with one projection view per eye.

A 2D desktop panel can be an `XrCompositionLayerQuad` referencing its own swapchain and specifying a pose and physical width/height in meters. The runtime composites projection and quad layers, then performs the headset’s final correction/presentation.

A quad is one panel image; it does not capture a Wayland surface, provide desktop semantics, or route clicks. Gnoblin still needs a surface-frame transport and an input bridge. Each frame must preserve the source surface’s pixel dimensions, damage revision, scale/transform, alpha, and synchronization.

Use bounded in-flight buffers, DMA-BUF format/modifier negotiation and explicit fences where supported; expose a measured copy fallback. Avoid one full desktop capture per window. Per-surface texture access is more efficient but requires a compositor-owned export path and careful protection/redaction policy. Mutter’s existing actor preview is not a stable universal surface-stream API.

For one OpenXR client as the primary session, the client can submit projection plus any number of quad layers supported by the runtime. This is the most portable OpenXR design. A separate “desktop overlay” joining an already running game needs `XR_EXTX_overlay` or a runtime-specific overlay API, and otherwise cannot assume it can add layers to another process’s frame.

If overlays are unsupported, run the desktop client as the primary session and make games/other XR apps secondary only where the runtime supports that mode.

The renderer should accept scene snapshots from Lua: each item names a Gnoblin surface/window ID, a frame revision, world pose, physical size, visibility, interaction mode and z-order. Lua computes docking to the moving viewport pill, free-space placement, focus, stacking and gesture policy. The renderer owns only OpenXR frame timing, swapchains, layer submission and texture sampling.

The compositor validates controller ray hits against the latest surface mapping revision and injects targeted pointer/button/key events through Gnoblin’s authorized input path. Layer-shell remains a 2D surface protocol: using a layer surface as the “pill” requires Gnoblin policy to bind that surface to a 3D anchor; it does not make the surface spatial by itself.

## Existing projects and lessons

### [Collabora `wxrc` / `wxrd`](https://gitlab.freedesktop.org/xrdesktop/wxrd)

- **Integration style:** `wxrc` is a wlroots OpenXR compositor; `wxrd` is a standalone XR Wayland desktop built around it. Apps launched inside that compositor are XR-native surfaces.
- **Lesson for Gnoblin:** A second Wayland session is a clean architecture for a dedicated XR desktop, but it does not mirror the existing 2D Mutter session.
    - `wxrd` documents the conflict between a wlroots DRM backend and a VR compositor both needing DRM master, and the difficult DMA-BUF modifier/import path.
    - It is a poor fit for making Gnoblin’s current windows spatial.
    - [Collabora design notes](https://www.collabora.com/news-and-blog/news-and-events/wxrd-a-standalone-wayland-compositor-for-xrdesktop.html)

### [xrdesktop and `gxr`](https://gitlab.freedesktop.org/xrdesktop/xrdesktop)

- **Integration style:** XR interaction libraries and compositor integrations for presenting and manipulating conventional desktop windows. Historical integrations included GNOME Shell/Mutter and KWin; `gxr` supplies graphics/runtime helpers.
- **Lesson for Gnoblin:** Closest precedent for “keep the 2D compositor, add an XR view.” It demonstrates the policy/input/window-manager integration is substantial even when OpenXR rendering is a separate library.
    - Check maintenance and exact current compatibility before reusing code.
    - [API reference](https://xrdesktop.freedesktop.org/doc/xrdesktop/master/ch01.html), [gxr](https://gitlab.freedesktop.org/xrdesktop/gxr)

### [WayVR (formerly wlx-overlay-s)](https://github.com/wayvr-org/wayvr)

- **Integration style:** Separate OpenXR/OpenVR overlay client for Monado, WiVRn and SteamVR. Captures selected desktop screens through PipeWire portals, renders movable/resizable panels, launches apps in VR and sends pointer/keyboard interaction.
- **Lesson for Gnoblin:** Strong reference for controller interaction and supporting several runtimes. Its screen selection and portal capture are not equivalent to low-overhead capture of every Mutter window. A separate overlay also inherits runtime multi-app constraints. The older [wlx-overlay-s](https://github.com/stuin/wlx-overlay-s) repository is a predecessor/fork; use the maintained WayVR project for current behavior.

### [Stardust XR](https://stardustxr.org/docs/dive-deeper/brief-overview)

- **Integration style:** XR display server with its own 3D scene/protocol and Wayland support for 2D applications.
- **Lesson for Gnoblin:** A coherent spatial compositor and 3D UI protocol can host both 2D and native 3D clients, but it is a different compositor/session.
    - Its split between ordinary Wayland surfaces and a Stardust-specific object protocol is a useful reference for defining Gnoblin’s spatial state boundary.

### Monado examples (`hello_xr`, `xrgears`, OpenXR simple example)

- **Integration style:** Sample OpenXR applications use the selected runtime to create sessions, render projection layers, submit frames and exercise graphics bindings.
- **Lesson for Gnoblin:** Useful for validating runtime selection, Vulkan bindings, frame timing and supported extensions. They demonstrate OpenXR application rendering, not Wayland window capture, desktop focus policy or a compositor integration. [Monado getting started and tools](https://monado.freedesktop.org/getting-started.html), [OpenXR demos](https://gitlab.freedesktop.org/monado/demos)

### [KWin XR work](https://github.com/Bake-Ware/kwin-vr)

- **Integration style:** Experimental KWin fork/VR mode work, including Monado DRM lease use for glasses and native compositor-level placement.
- **Lesson for Gnoblin:** Shows the deepest integration route: window manager directly owns 3D placement and all clients work as windows. Cost is a large compositor fork, graphics/input/lifecycle complexity and version coupling. It is a caution against moving the whole VR renderer into Mutter merely to gain scene ownership. Treat status as experimental, not a stable upstream API.

### Mutter / GNOME

- **Integration style:** Mutter added `wp_drm_lease_v1` support in GNOME 47.
    - Mutter also has nested/devkit modes and PipeWire screen-cast infrastructure, but GNOME Shell and Mutter do not provide a general OpenXR window-space scene.
- **Lesson for Gnoblin:** DRM leasing is a small, established bridge for wired HMD display ownership, not an OpenXR rendering integration.
    - Do not rely on Steam’s Linux support page as a current statement about Mutter: GNOME 47 release notes explicitly document the protocol.
    - Check the exact Mutter 51 build, connector policy and runtime path.
    - [GNOME 47 DRM lease notes](https://release.gnome.org/47/developers/), [Mutter devkit](https://github.com/GNOME/mutter/blob/main/doc/building-and-running.md)

### Hyprland experiments

- **Integration style:** I found no primary source for a maintained native XR scene/window-management mode in Hyprland. It can be a Wayland host for separate OpenXR overlays such as WayVR, subject to its capture and input paths.
- **Lesson for Gnoblin:** Distinguish host-compositor compatibility from compositor-owned spatial windows. A sidecar overlay does not make Hyprland’s windows native 3D surfaces or grant arbitrary surface capture. [WayVR](https://github.com/wayvr-org/wayvr)

### [Picom xrdesktop companion](https://github.com/yshui/picom-xrdesktop-companion)

- **Integration style:** Separate process beside an X11 compositor; mirrors windows in XR.
- **Lesson for Gnoblin:** Illustrates a sidecar presentation client, but its X11 capture/window-stack assumptions do not translate directly to Wayland’s security model.

### Wayland-in-VR generally

- **Integration style:** Usually either a separate XR Wayland compositor (wxrd/Stardust) or a sidecar overlay that captures screens/windows (WayVR/xrdesktop integrations).
- **Lesson for Gnoblin:** Wayland clients cannot inspect or capture arbitrary other clients.
    - A supported compositor API or portal authorization is necessary.
    - Existing protocols such as `wlr-screencopy` are compositor-specific and typically output-oriented; they do not supply universal per-window capture, 3D geometry, focus, or targeted input.

## Gnoblin/Mutter boundary

The minimum viable route is a shell-independent OpenXR client plus narrowly scoped Mutter capabilities, with Lua retaining all policy:

1. **Use existing DRM leasing for wired HMDs.** GNOME 47 documents `wp_drm_lease_v1`; check the exact pinned Mutter 51 implementation and that Gnoblin’s output policy advertises only eligible non-desktop headset connectors. Add only a small policy/configuration hook if Gnoblin needs to classify or expose a leasable connector differently. Network-streamed headsets such as WiVRn do not need this path.
2. **Expose per-surface GPU frames.** The current Lua/API designs describe window previews, not a continuously synchronized stream API. Add a compositor-owned frame export with buffer lifetime, damage, transform, alpha, synchronization, revoke, and protected-content behavior. Prefer DMA-BUF with modifiers/fences and have the XR renderer import on the runtime-selected GPU; measure any cross-GPU/copy route.
3. **Expose validated scene and input operations to Lua.** Reuse stable window IDs, events, operation handles, and authorized input policy. Add only XR-specific facts/operations (runtime state, tracked poses or ray input, surface frame revisions, 3D transforms) that Lua cannot derive safely. Do not expose Mutter C objects or let the XR process focus/inject input directly.
4. **Keep XR lifecycle outside the Mutter frame loop.** A restartable user service/client handles OpenXR instance/session/device changes. Lua selects placement and response to XR state; the renderer reports bounded events and failures over the existing local control boundary. A runtime crash or headset disconnect must revoke input and retain or restore 2D output state.
5. **Keep 2D mode first-class.** The headset is an `xr` output kind, but streamed headsets may have no local DRM connector. Output state should distinguish physical leased outputs, network XR sessions and virtual/headless outputs. Layer-shell surfaces are Lua-selected scene objects attached to a tracked viewport anchor; applications continue using ordinary Wayland surfaces.

This follows Gnoblin’s existing ownership rules in [design/lua-api.md](../lua-api.md), [design/lua-input-policy.md](../lua-input-policy.md), [design/monolithic-compositor.md](../monolithic-compositor.md), and the output design’s reserved `xr` kind. The current design explicitly leaves shell presentation to Wayland clients and treats window previews as bounded snapshots; sustained per-window frame export is new work and needs independent lifecycle/security semantics.

## Risks and unresolved questions

- **Capture cost and latency:** copying/compositing many window textures, then encoding or importing them, can consume GPU bandwidth and memory. Establish an idle/damage budget and measure one headset at target refresh before promising continuous all-window mirroring.
- **GPU identity and synchronization:** the OpenXR runtime may choose a different GPU from Mutter. Vulkan external-memory handles, modifiers, semaphore FDs and explicit synchronization must agree; cross-GPU import can fail or trigger copies.
- **OpenXR overlay portability:** `XR_EXTX_overlay` is not a safe baseline. Monado requires service mode for it; SteamVR/WiVRn support must be checked per installed runtime/version. Do not promise simultaneous game-plus-desktop layers without testing.
- **DRM lease policy:** Mutter’s protocol support does not guarantee that a particular headset connector is exposed, leasable, on the same GPU, or accepted by a runtime.
    - Steam’s Linux support documentation is stale about GNOME’s post-47 protocol; test the actual connector/runtime combination.
- **Protected and private content:** compositor export must respect capture restrictions and screen-lock/privacy state. A same-user sidecar is not automatically a security boundary; authorization stays in Gnoblin.
- **Input correctness:** controller rays need a stable mapping from the rendered frame revision to the surface-local pointer coordinate. Tracking loss, stale frames, focus changes and concurrent gestures must cancel held buttons/drags.
- **Surface semantics:** popups, subsurfaces, transforms, fractional scale, cursor, IME, drag-and-drop, menus and layer-shell exclusive zones need explicit XR behavior. Capturing only top-level textures will produce an incomplete desktop.
- **Session and hotplug lifecycle:** runtime switching, headset sleep/reconnect, service crashes, lock/logout, suspend/resume and unplug during lease need clean recovery and no stuck input grab.
- **Text legibility and physical size:** a quad’s size is in meters; pixels alone do not define readable text. DPI, per-window scaling, distance and angular resolution need real-headset measurements.
- **Project maturity:** xrdesktop and several KWin XR experiments are older or experimental. Their architectural evidence remains useful, but reusing code would require checking current maintenance, license, supported Mutter/KWin versions, and device coverage.
- **Open design decision:** choose whether initial VR is a single primary OpenXR session (portable quad layers, can exclude concurrent games) or an overlay joining another app (needs `XR_EXTX_overlay`/runtime-specific support).
    - For the first Gnoblin slice, prefer primary-session mode and add overlay coexistence only after a runtime matrix proves it.
