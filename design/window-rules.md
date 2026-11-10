# Window rules: legacy contract, Rust target and gap

Status: proposed. Base: `origin/rust-compositor` at `2f2ff7c5b`. Nothing here is built yet.

Window rules are the user's main way to change how windows look and where they open. The legacy compositor
(C, under `src/`) honours the full contract in `docs/config/window_rule.md`. The Rust tree
(`crates/gnoblin-wm`) honours only a small part of it. This document fixes the contract, says where the Rust
tree gets each piece, and lists the gaps and the choices that need Kieran.

## 1. Legacy contract

Sources: `docs/config/window_rule.md`, `docs/guides/window_rules.md`, `docs/guides/window_effects.md`,
`docs/guides/window_frames.md`, `docs/guides/shaders.md`, `docs/guides/window_state_shortcuts.md`. Code:
`src/control/windows/gnoblin-control-window-rules.c` (matching at 558-665, apply at 1103), validator
`src/config/gnoblin-config-windows.c` (464-547), pattern engine `src/config/lua/gnoblin-lua-pattern.c`, Lua
binding `src/config/lua/gnoblin-lua-install.c:311`.

### 1.1 Match keys

Every supplied key must match. An omitted key does not constrain. An empty `match` matches every window.

| Key                | Values                                                                 | Doc                      | Legacy code              |
| ------------------ | ---------------------------------------------------------------------- | ------------------------ | ------------------------ |
| `type`             | `"window"` (application window), `"layer"` (layer-shell surface)       | config 19; guide 22-35   | `window-rules.c` 558-665 |
| `focused`          | boolean                                                                | config 20; guide 22-27   | same                     |
| `app_id`           | Lua pattern. GTK app ID, falling back to WM class when GTK ID is empty | config 21; guide 22-35   | same; validator 521      |
| `title`            | Lua pattern against current title                                      | config 22                | same                     |
| `layer`            | Lua pattern against layer-shell namespace                              | config 23                | same                     |
| `workspace_id`     | exact, case-sensitive ID; declared or runtime-assigned                 | config 24, 59-66         | same                     |
| `workspace_number` | integer 1-1024, current one-based position                             | config 25                | same                     |
| `client_type`      | `"wayland"` or `"x11"` (Xwayland); validator rejects other values      | config 26; validator 508 | same                     |

Aliases in the legacy code (`app-id`, `workspace-id`, `workspace-number`, `client-type`, `blur-ignore-shadows`)
are read in `native_rule_lookup_alias` (window-rules.c 66). The Rust target does not carry aliases. See gap
table, row 15.

Pattern rules (config 27-30, guide 92-120): Lua 5.4 `string.find` syntax, case-sensitive, unanchored by default.
`.` matches any character, `%.` a literal dot, `^`/`$` anchor. JavaScript or PCRE syntax is not supported.
Patterns are checked at config load (validator 521-530). A bad pattern is a config error that names the rule
and field.

### 1.2 Rule fields

| Field                 | Type and range                                                                              | Default                            | Doc                                         | Legacy code                             |
| --------------------- | ------------------------------------------------------------------------------------------- | ---------------------------------- | ------------------------------------------- | --------------------------------------- |
| `blur`                | number 0-100 (px radius); 0 disables                                                        | 0                                  | config 50; validator 373-390                | window-rules.c 1103+                    |
| `opacity`             | number 0-1, content and text                                                                | 1                                  | config 51                                   | same                                    |
| `blur_ignore_shadows` | boolean                                                                                     | false                              | config 52; validator 392-398                | same                                    |
| `corners`             | table, see 1.3                                                                              | see 1.3                            | config 53, 81-96                            | same                                    |
| `shader`              | GLSL path; `""` clears                                                                      | none                               | config 54; shaders.md 23-53                 | `native_window_shader_resolve_path` 887 |
| `shader_uniforms`     | up to 64 names to finite numbers; `gnoblin_` prefix reserved                                | none                               | config 55; shaders.md 38-53                 | same                                    |
| `animation`           | name, or map with `open`, `close`, `layer_open`, `layer_close`, `duration` 0-5000, `easing` | none                               | config 56, 116-131                          | same                                    |
| `workspace`           | exactly one of `id` (non-empty, at most the length limit) or `number` 1-1024                | none                               | config 57; guide 155-220; validator 404-458 | `native_rule_workspace_target` 940      |
| `frame`               | table, see 1.4                                                                              | `mode` auto on X11, off on Wayland | config 58, 135-147; frames guide            | `frame_policy` in 1103                  |

`borders` is rejected with a message that points at `corners.border_width` (validator 480-485). Unknown keys
are not listed as errors in the doc; the Rust target refuses them by name (section 4).

### 1.3 Corners and shadows

From config 81-114:

- `corners.radius` 0-200, default 0. `corners.smoothing` 0-1, default 0.
- `corners.mode` `auto` (default), `force`, `off`. `off` also disables border and shadow.
- `corners.padding` four numbers, top right bottom left, each finite and in -128..128. Default `{0,0,0,0}`
  (validator 340-368).
- `corners.keep_maximized` (true), `keep_fullscreen` and `keep_tiled` (false), `skip_libadwaita` (true),
  `skip_libhandy` (false), `remove_csd` (false), `keep_shadow` (false).
- `corners.border_width` -40..40, default 0. `corners.border_color` `#RRGGBB[AA]`, default `#808080ff`.
- `corners.shadow`: `false`, `true` (default shadow), one table, or a list of 1-4 layers. Table fields
  `x`, `y`, `spread` default `0`, `4`, `4`, range -100..100; `blur` default 28, 0-100; `opacity` default 0.6,
  0-1; `color` default `#000000ff`.
- `corners.shadow_animation`: `animation` (must support `shadow-change`), `duration` 0-2000 ms (default 0),
  `easing` (same set as `gnoblin.configure`).

### 1.4 Frame

From config 135-147 and `docs/guides/window_frames.md`:

- `mode`: `off`, `auto`, `prefer-server`, `replace`.
- `extents`: four integers 0-256, top right bottom left, default `{32,1,1,1}`.
- `crop`: four integers, same order and range, default `{0,0,0,0}`.
- `renderer` (default `"native"`, a registered service name), `style` (default `"default"`),
  `background` `#242424`, `foreground` `#eeeeee`, `inactive_background` `#303030`.
- `button_layout`: list of `minimize`, `maximize`, `close`, no duplicates; `{}` hides buttons.
- `client_type = "x11"` in `match` gives X11 apps their own frame rule (frames guide 161).

### 1.5 Animation

`animation` (config 116-131) names a built-in or registered animation. Events: `open`, `close`, `dialog_open`,
`dialog_close`, `layer_open`, `layer_close`, `minimize`, `restore`, `workspace_switch`, `shadow_change`,
`resize`. Per-rule `duration` and `easing` override the selected animation.

### 1.6 Ordering and precedence

Authoritative: guide 221-227 and the apply loop at window-rules.c 1103-1200.

1. Configured rules are walked in config order. A rule that does not match is skipped. A matching rule
   overwrites only the fields it sets. Earlier values survive otherwise.
2. The walk does not stop at the first match.
3. Effects set at runtime by Lua for one window are one more rule, applied last, with no match
   (window-rules.c 1103-1200, `gnoblin_control_window_effect_rule`).
4. Passing a full `window_rules` list to `gnoblin.configure` replaces the configured list.
   `gnoblin.window_rule { ... }` appends.

### 1.7 When rules apply

- Placement (`workspace`) runs when a window is first managed (`gnoblin_control_place_window_by_rules`, 967).
- Appearance rules run on the window's surface. Legacy re-runs every rule on any GObject `notify` from the
  window, which includes title and app_id changes, and on position, size and workspace changes
  (`gnoblin-control-signals.c` 548-605, 150-158). The Rust tree runs rules once, on first management, so gap row
  14 and decision 2 cover the change.

## 2. Rust target

### 2.1 Where each piece lives

| Piece           | Home                                                                                                       | Notes                                                                                                                                       |
| --------------- | ---------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------- |
| Pattern matcher | `crates/gnoblin-wm/src/rules.rs`                                                                           | Port Lua 5.4 `string.find` patterns (decision 1) with a work budget against runaway matches.                                                |
| Match           | `rules.rs`                                                                                                 | Add `type`, `focused`, `layer`, `workspace_id`, `workspace_number`, `client_type`. Keep `kind`, `app_id`, `title`.                          |
| Rule and merge  | `rules.rs`                                                                                                 | `Rule { matcher, fields }`, where each field is `Option<T>`. Merging walks all rules in order and sets only fields that are `Some`.         |
| Effects         | `rules.rs` builds the list; `effect.rs` and `gnoblin-render` keep `Effects`                                | D26. The rule emits an ordered `Vec<EffectPass>`.                                                                                           |
| Application     | `wm.rs` `apply_rules` (1157), called on first management and on title or app_id change                     | Emits `Effect`s as now.                                                                                                                     |
| Lua binding     | `crates/gnoblin-lua`: `gnoblin.window_rule { ... }`                                                        | Validates the table, returns an `Operation` (lua-api-goal). Refuses unknown keys by name.                                                   |
| Config schema   | `crates/gnoblin-config/schema/config.toml`, `window-rules` section (currently `carried = true`, line 2209) | Replace `carried` with a typed record: rule list, match record, effects array, frame record. Codegen emits the Rust record and validation.  |
| Window object   | `gnoblin-lua` window member (`host_members.toml`)                                                          | Read-only `window.effects` list of `effect` objects (D26). Read-only `window.matched_rules`, a list of rule indexes (proposal, decision 6). |

### 2.2 Effects mapping (D26)

Legacy flat keys are replaced by an ordered `effects` array of one-key tables:

| Legacy                                                                          | Target                                                    |
| ------------------------------------------------------------------------------- | --------------------------------------------------------- |
| `blur = 24`                                                                     | `effects = { { blur = 24 } }`                             |
| `shader = "p.glsl"`, `shader_uniforms`                                          | `effects = { { shader = "p.glsl", uniforms = { ... } } }` |
| `corners.shadow = {...}`                                                        | `effects = { { shadow = {...} } }`                        |
| `opacity`, `blur_ignore_shadows`, `corners.*` (radius, border, padding, keep_*) | unchanged, flat fields (not stackable passes)             |

`opacity` is a node property (D24), not a pass. `blur_ignore_shadows` modifies the blur pass and stays a
flat field. The stack has a limit of 32 passes. A pass the renderer cannot draw is skipped and logged, not an
error (D26).

### 2.3 Error model

- Unknown key: refused at config load. The error names the rule index and the key, for example
  `window-rules[3].foo: unknown key`.
- Wrong type or range: refused at config load, naming the field and the allowed range.
- Empty `match` is allowed (matches everything), as in legacy.
- A field this build cannot honour (section 4): refused at config load, naming the field and the missing
  prerequisite. Never silently ignored.
- A pass the active renderer cannot draw (D26): accepted, skipped at render, reason on the `effect` object and
  in the log. Not an error.
- A bad pattern: refused at config load, naming the rule, field and position of the error.
- Invalid entry in `effects`: the entry is dropped with a named problem and the rest of the rule loads (D26).
- Over the rule count cap (decision 4) or pass cap (32): refused at load, naming the count.

## 3. Gap table

Effort: S (hours), M (a day or two), L (more than a few days). Layer: core (`gnoblin-wm` and friends),
lua (`gnoblin-lua` host), shell (shell-side consumers). "Honour now?" says whether this build can apply the
field. The Rust tree has no legacy window-rule code, so the first column is the legacy contract.

| #   | Field / key                                    | Legacy                                     | Rust today                                                  | Effort | Layer       | Honour now? |
| --- | ---------------------------------------------- | ------------------------------------------ | ----------------------------------------------------------- | ------ | ----------- | ----------- |
| 1   | `match.type` window/layer                      | yes                                        | window only (`Match.kind`, `WindowKind`); no layer surfaces | M      | core        | window only |
| 2   | `match.focused`                                | yes                                        | none                                                        | S      | core        | no          |
| 3   | `match.app_id` (+ WM class)                    | yes, WM class fallback                     | `app_id` pattern; no WM-class fallback                      | S      | core        | partly      |
| 4   | `match.title`                                  | yes, Lua pattern                           | `title` pattern, quantifier-free                            | M      | core        | partly      |
| 5   | `match.layer` namespace                        | yes                                        | none (layer surfaces not in `gnoblin-wm`)                   | M      | core+layers | no          |
| 6   | `match.workspace_id`                           | yes                                        | none                                                        | S      | core        | no          |
| 7   | `match.workspace_number`                       | yes                                        | none                                                        | S      | core        | no          |
| 8   | `match.client_type`                            | yes                                        | none; Window has no X11 flag                                | S      | core        | no          |
| 9   | `workspace` (id or number)                     | yes                                        | `RuleAction::PlaceOnWorkspace(WorkspaceSelector)`           | S      | core        | yes         |
| 10  | `maximise` action                              | not in legacy contract                     | `RuleAction::Maximise`; no config key                       | S      | core+lua    | decide (6)  |
| 11  | `blur`, `blur_ignore_shadows`                  | yes                                        | none                                                        | M      | core+render | no          |
| 12  | `opacity`                                      | yes                                        | none                                                        | S      | core+render | no          |
| 13  | `shader`, `shader_uniforms`                    | yes                                        | none (needs WASM wiring)                                    | L      | core+render | no          |
| 14  | re-match on title/app_id change                | yes (window-rules.c 369-400, 1020-1100)    | none; rules run once on first management (wm.rs 1152 doc)   | S      | core        | no          |
| 15  | aliases (`app-id` etc.)                        | yes                                        | none                                                        | S      | lua         | no          |
| 16  | `corners.radius/smoothing/mode/padding/keep_*` | yes                                        | none                                                        | L      | core+render | no          |
| 17  | `corners.border_width/color`                   | yes                                        | none                                                        | M      | render      | no          |
| 18  | `corners.shadow` (+ layers)                    | yes                                        | none; becomes `shadow` pass (D26)                           | L      | core+render | no          |
| 19  | `corners.shadow_animation`                     | yes                                        | none                                                        | L      | core+anim   | no          |
| 20  | `corners.remove_csd`                           | yes                                        | none                                                        | L      | render      | no          |
| 21  | `animation` (+ map)                            | yes                                        | none (animation system lives in the layers work)            | L      | core+anim   | no          |
| 22  | `frame` (all subfields)                        | yes                                        | none (layers phase 6)                                       | L      | shell+core  | no          |
| 23  | shaped-window `alpha/input/both`               | not legacy; decided (rust-compositor 3034) | none                                                        | M      | render      | no          |
| 24  | `window.effects` read and `add/set/remove`     | not legacy; D26                            | `effect.rs` ordered list, no Lua surface                    | M      | lua+core    | no          |
| 25  | `gnoblin.window_rule` binding                  | yes (install.c 311)                        | none (no `window_rule` in `gnoblin-lua`)                    | M      | lua         | no          |
| 26  | config schema record                           | `window-rules` (validator 464-547)         | `carried = true`, untyped (schema line 2209)                | M      | config      | no          |
| 27  | Lua pattern matcher                            | yes, bounded (`gnoblin-lua-pattern.c`)     | quantifier-free `Pattern`                                   | M      | core        | partly      |

Summary of what the Rust tree honours today: placement on a workspace (row 9) and maximise (row 10). It matches
only `kind`, `app_id` and `title`, with a pattern language that differs from the documented one. Everything
visual is missing. Rules run once, on first management.

Rows the Rust tree cannot honour in this build (refuse at config load, section 2.3): 2, 5, 6, 7, 8, 11-13,
15-22, 23 once the renderer has no shaped-window path.

## 4. Field refusal list

Until a field's row is done, the config loader refuses it with a message that names it and its prerequisite.
Expected messages:

- `window-rules[N].blur: effects need the blur pass (render), not yet built in this compositor`
- `window-rules[N].frame: frames need the layers phase 6 surface renderer, not yet built`
- `window-rules[N].shader: shaders need the shader pass (render), not yet built`
- `window-rules[N].animation: animations need the animation system, not yet built`

Refusal is a load error. The rule list is not partly applied. Losing a rule silently is the failure this
prevents.

## 5. Slices

Each slice is gated on Agent37 (`~/dev/lanes/a37-gate.sh`), tests first, failure modes first.

1. Matcher: port Lua 5.4 `string.find` patterns into `rules.rs` with a work budget. Tests: anchors, `%` classes,
   `-` `*` `+` `?` quantifiers, bad pattern names the position, a pathological pattern stops at the budget.
2. Match and merge: add `focused`, `type`, `client_type`, workspace matchers, field-wise merge of `Rule`. Tests:
   later rule overrides only the fields it sets; empty match; every set key must hold; unknown key refused by
   name.
3. Schema record: type the `window-rules` section in `config.toml`, codegen, validation with named errors.
4. Lua binding: `gnoblin.window_rule { ... }` in `gnoblin-lua`, returns `Operation`, refuses unknown keys.
5. Application: re-match on title and app_id change (decision 2); window object reports matched rules.
6. Effects: `effects` array and pass stack (D26), once `effect.rs` and render passes exist.
7. Docs: `docs/config/window_rule.md`, `docs/shell-api/overrides.md`, `docs/shell-api/windows.md`, generated Lua
   annotations, to match the real behaviour once each slice lands.

## 6. Decisions

Decided 2026-11-07 by Kieran, delegated to Main; these replace the open questions.

1. **Pattern engine.** Port the legacy Lua 5.4 matcher with a work budget, using the shared `gnoblin-lua-pattern`
   crate. Existing configs keep working, including `-`, `*`, `+` and `?`.
2. **Re-match on title or app_id change.** The merge is defined per window so the result is stable across repeated
   matches.
3. **Effects list (D26) accepted.** `effects` replaces the flat `blur` and `shader` keys. Flat keys are removed, not
   kept as sugar. The docs carry a migration note.
4. **Rule cap.** 256 rules per list.
5. **Unsupported fields refused at load.** The loader names the field and its prerequisite.
6. **`maximise` stays a config key** (`maximise = true`). `window.matched_rules` is not added.
