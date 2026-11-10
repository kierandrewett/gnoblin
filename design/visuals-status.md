# Visuals status

Status of window visual effects per renderer, read from the code at `c451b05de`
plus the software corner clip (`994f47406`). Status is from reading code unless a
row says a test was run. "Read" means the test body was read, not executed.

Decisions these rows rest on: D24 (blur follows opacity), D26 (ordered effect
stack), and the shaped-windows amendment in `design/rust-compositor.md`.

## Matrix

| Effect                      | Software (`render-software`)   | GLES (`render-gles`)                          | Vulkan (`render-vulkan`)       |
| --------------------------- | ------------------------------ | --------------------------------------------- | ------------------------------ |
| Rounded corners             | Done (this lane), see below    | Done, `preamble.wgsl:56`                      | Done, `plan.rs:142,162`        |
| Shadow                      | Absent                         | Done, `Pass::Shadow` at `gles/src/lib.rs:953` | Absent, `vulkan/src/lib.rs:24` |
| Opacity                     | Done, alpha blend (not traced) | Done, group opacity at `lib.rs:1097`          | Done, `Layer` carries opacity  |
| Tint                        | Done, per-channel multiply     | Done, `view.wgsl:55`                          | Done, `tests.rs:283`           |
| Blur                        | Absent                         | Done, `Pass::Blur` at `lib.rs:1000`           | Absent, `vulkan/src/lib.rs:24` |
| Blur follows window opacity | Absent (no blur)               | Done, `lib.rs:771,1097`                       | Absent                         |
| User shaders                | Absent                         | Mechanism only, see below                     | Absent                         |
| Shaped windows (silhouette) | Absent                         | Absent, `gnoblin_sdf` is one radius           | Absent                         |
| Native frames               | Protocol, renderer-agnostic    | Same                                          | Same                           |

"Done" for a renderer means the draw path exists and has a named test (below).
"Absent" means no code path for it was found.

### Evidence, by row

- **Rounded corners, software.** `gnoblin-render-software/src/lib.rs`: clip in
  `paint()`, `corner_spans`, `clip_to_shape`. Tests (all 4 pass on Agent37; the
  crate has 20 tests): `a_rounded_solid_reveals_the_background_at_its_corners`,
  `a_rounded_surface_clips_the_client_pixels_and_keeps_the_middle`,
  `a_corner_radius_is_logical_and_scales_with_the_output`,
  `an_oversized_radius_is_clamped_to_a_circle`. The corner is hard-edged, not
  antialiased.
- **Rounded corners, GLES.** Shader `gnoblin_corner` in
  `render-frame/src/shader/wgsl/preamble.wgsl:56`, used by `solid.wgsl:11` and
  `surface.wgsl:12`. Radius clamp `gnoblin-render-gles/src/lib.rs:801`. Test
  `tests.rs:215` `a_corner_radius_reveals_the_background` (read).
- **Rounded corners, Vulkan.** `plan.rs:142,162` sets `uniforms.corner`. Test
  `vulkan/src/tests.rs:463` `a_rounded_corner_shows_what_is_behind` (read). Not
  executed here.
- **Shadow, GLES.** `Pass::Shadow` (`render/src/effect.rs`), draw at
  `gles/src/lib.rs:953`. Tests `tests.rs:247`
  `a_shadow_darkens_next_to_the_edge_and_fades` and `:289`
  `a_shadow_does_not_leave_the_damage` (read). Shared extent code:
  `render/src/effect.rs:813` (`a_shadow_extends_the_node_bounds_by_its_offset_and_blur`),
  `:924` (spread), `:832` (3D offset refused by name).
- **Shadow, software.** No shadow pass in `render-software/src`; grep found none.
- **Opacity.** Software: alpha blended in the paint path; not traced to a test.
  GLES: group opacity at `lib.rs:1097`; isolated-group test `tests.rs:1111`.
  Vulkan: test `vulkan/src/tests.rs:268`
  `a_layer_opacity_of_a_half_matches_the_software_renderer` (read).
- **Tint.** Software: per-channel multiply; a tinted layer is copied out of the
  client buffer first (`render-software/src/lib.rs:26-27`). GLES: `gnoblin_tint` at
  `render-frame/src/shader/wgsl/view.wgsl:55`; test `tests.rs:185`
  `a_tint_multiplies_the_layer` (read). Vulkan: test `tests.rs:283`
  `a_tint_matches_the_software_renderer` (read).
- **Blur, GLES.** `Pass::Blur`, draw at `gles/src/lib.rs:1000`. Tests `tests.rs:367`
  `a_blur_blurs_a_sharp_edge_behind_the_layer`, `:390`
  `more_blur_passes_spread_the_edge_further`, `:434`
  `a_blur_at_the_edge_repeats_the_edge` (read). Also `:416`
  `a_blur_tint_grades_the_blurred_backdrop` (read).
- **Blur follows opacity, GLES.** Blur drawn inside the node's opacity group, so it
  fades with the layer (`lib.rs:771,1097`). Test `tests.rs:1139`
  `a_blurred_node_fades_monotonically_with_group_opacity` (read). Confinement test
  `tests.rs:1127` `a_blur_reaches_nothing_outside_its_node` (read).
- **User shaders, GLES.** `UserShader` at `lib.rs:149`, `set_user_shader` at
  `lib.rs:346`, `Pass::Shader` draw at `lib.rs:1200`. Tests `tests.rs:470`
  `a_user_shader_shades_its_layer`, `:503` `a_user_shader_reads_its_uniforms`, `:546`
  `a_user_shader_refuses_a_mistyped_uniform`, `:590`
  `a_broken_user_shader_leaves_the_frame_drawn` (all read).
  **No production caller of `set_user_shader`.** No crate outside
  `gnoblin-render` calls it. There is no config loader.
- **User shaders, Vulkan.** Only Vulkan's own shader compile tests
  (`shader.rs:99`, `tests.rs:632`). Not user shaders.
- **Shaped windows.** No silhouette or mask code found in `crates/`. The frame
  shader's `gnoblin_sdf` (`preamble.wgsl:74`) is a rounded box with one radius.

### Shared code

`gnoblin-render` (`effect.rs`, `scene.rs`) owns the effect stack and extents.
`gnoblin-render-frame` owns the frame shaders and uniforms (`shader/uniforms.rs`).
`frame.rs:634` drops an unusable corner radius (test
`an_unusable_corner_radius_is_dropped`, read).

Production corner radius is set by `Effect::Appearance` in
`gnoblin-wayland/src/state.rs`, which calls `set_window_corner_radius`
(`gnoblin-backend/src/render/compose.rs`), from a `corners.radius` window rule.

### Native frames (protocol)

`crates/gnoblin-wayland/src/protocols/gnoblin_window_frame_v1.rs` implements the
frame protocol. `tests/window_frame.rs` covers:

- an ordinary client sees no frame protocol (`:258`);
- a renderer is offered only its own protocol (`:271`);
- a mapped window is offered a frame and presents it (`:289`);
- protocol errors for a roled surface, a second surface, an unknown serial, a wrong
  size, an out-of-range region and too many regions (`:342` to `:430`);
- hover is reported to the renderer (`:450`);
- pointer routing over overlapping windows (`:551`) and tiled windows (`:610`).

Status: done and tested as read. This lane did not run these tests.

**Starter config: loads.** `data/default-config/config/10-windows.lua` sets an
`opacity` rule and a `corners = {radius = ...}` rule, both accepted. The starter
load test passes on Agent37 after `a9153b70b`, which fixes the spellings and
removes the refused `frame` rule.

## Config keys

| Key                                                                                          | Where                                                | Status                                                                          |
| -------------------------------------------------------------------------------------------- | ---------------------------------------------------- | ------------------------------------------------------------------------------- |
| `blur` (enum `Blur`, `setting = "blur"`)                                                     | `crates/gnoblin-config/schema/config.toml:29-32`     | Schema only (`auto`, `on`, `off`). Renderer use not traced.                     |
| `frame-renderers`                                                                            | `schema/config.toml:2132`                            | Live. Parsed in `document.rs:164`, `hooks.rs:687`.                              |
| Window rule `blur`, `blur-ignore-shadows`, `shader`, `shader-uniforms`, `animation`, `frame` | `hooks.rs:126-135`                                   | Refused at load, each with a reason. Asserted by `window_rules.rs:86`.          |
| Window rule `match.layer`                                                                    | `hooks.rs:172-176`, `docs/config/window_rule.md:106` | Refused (layer-shell selector not built).                                       |
| Window rule `corners` table                                                                  | `schema/config.toml` (RuleCorners)                   | `radius` accepted (>= 0). Other keys, such as `shadow`, are refused as unknown. |
| `tint` (window rule)                                                                         | none                                                 | No key in the schema or the refused list.                                       |
| Per-window `effects` list (D26)                                                              | `design/rust-compositor.md` D26                      | Proposed, not built. No `effects` key in the schema.                            |
| Corner radius from config                                                                    | `corners.radius` window rule                         | Live. Applied by `Effect::Appearance` (`gnoblin-wayland/src/state.rs`).         |

## Docs coverage

- `docs/guides/window_effects.md`: rewritten in this lane. It lists `opacity` and
  `corners.radius` as accepted, and the rest of the window-rule effects as refused
  or planned.
- `docs/config/window_rule.md`: the refused-keys table matches `hooks.rs:126`.
  `opacity` and `corners.radius` are documented as accepted.
- `docs/guides/shaders.md`: status note added; the `shader` window-rule example is
  refused at load.
- `docs/guides/theming.md:65`: pointer corrected to the window effects guide.
- `docs/background-effects.md`: the `ext_background_effect` blur-region protocol,
  separate from window rules. Not audited in detail.

## Gaps, in order

1. Corner radius and opacity are settable from window rules (`corners.radius`,
   `opacity`). The starter loads. The corner shadow keys and `frame` stay refused.
2. Shadow, blur, user shader and shaped windows have no config path. D26 (the
   `effects` list) is the planned route. Only GLES draws shadow, blur and shader.
3. Shaped windows: no silhouette source is chosen (the D26 amendment lists buffer
   alpha, opaque region, input region, radius). Not started.
4. Parity follow-up: software shadow, blur and shader; Vulkan corners beyond the
   existing test. Not built in this lane.
