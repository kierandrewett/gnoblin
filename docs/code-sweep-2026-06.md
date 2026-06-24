All claims verified. I have enough to produce an accurate, actionable report. The findings are high-quality and the line numbers check out. Let me write the synthesis.

# Code-Quality Sweep — Action Report (gnoblin/src)

142 raw findings from ~22 reviewers, merged and deduplicated down to **~55 actionable items** across 8 themes. I spot-verified every high/medium item below against the actual source (line numbers confirmed). Severity/effort are the reviewers' calls, re-checked.

---

## TOP 10 — do these first (highest value-per-effort)

1. **Fix the C/Rust config parser drift on empty keys** — `src/clients/shell-ui/src/config.rs:138` stores `= value` lines under an empty-string key; the C parser (`gnoblin-config.c:155`) explicitly skips them. Add `if key.is_empty() { continue; }` + a parity test. *Behavioural divergence, the MEMORY note warns about exactly this.*
2. **Delete the contradictory dock comment block** — `src/compositor/gnoblin-rules.cpp:610-616` (un-frosted) flatly contradicts `617-620` (frosted), no code between them; code path frosts. Delete `610-616`.
3. **Route the top-level "actor" JSON through the existing non-finite guard** — `src/compositor/gnoblin-control.cpp:712-723` emits `%.0f/%.3f` raw on actor pos/size/scale, bypassing `json_fvec`; an unmapped actor mid-animation prints `inf`/`-nan` and breaks the whole inspector parse. Use `json_fvec` + `isfinite` like `dump_actor_tree` already does.
4. **Extract spec-parser tokenizer to `gnoblin-spec-util.{c,h}`** — `skip_spaces`/`at_end`/`parse_int`/`parse_whole_int` are byte-identical in `gnoblin-rules-spec.c:10-45`, `gnoblin-actions-spec.c:11-61`, `gnoblin-output-spec.c:11-34`. One shared copy kills the largest C dedup cluster. *(verified identical)*
5. **Collapse `apply_theme` across 7 Rust clients into a macro** — verbatim copies at osd:23, power-menu:59, window-menu:86, notifyd:87, launcher:148, topbar:134, dock:62. Add `apply_shell_theme!($component)` next to the existing `apply_shell_chrome_to_theme!` in `shell-ui/src/lib.rs:84`. *(verified)*
6. **Gut the half-dead `quicksettings.rs`** — `src/clients/shell-ui/src/quicksettings.rs` runs a full BlueZ `GetManagedObjects` walk + power-profile read **every 2s** to fill `bt/power_mode/volume/mic/wifi_name` that nothing reads (topbar reads only `wired/wifi/muted`, confirmed at main.rs:243-254). `cycle_power_profile`/`bluetooth`/`power_profile` have zero callers. Real recurring cost. *(verified)*
7. **Reset the output retry budget per apply-cycle** — `src/compositor/gnoblin-output.cpp:248` `static int retries` only ever increments; once startup burns the 8 retries, every future hotplug gets zero. Reset `retries = 0` at the top of `gnoblin_output_apply()`. *(verified)*
8. **Make foreign-toplevel ACTIVATED read `appears-focused`** — `meta-wayland-foreign-toplevel-management.c:133` reports ACTIVATED from `meta_window_has_focus()` but only subscribes to `notify::appears-focused` (line 383). Different focus notions → missed/spurious updates. Read `appears-focused` to match the trigger. *(verified)*
9. **Stop the night-light gamma failure re-loop** — `src/clients/night-light/src/main.rs:239` records no "gave up" state on `Failed`, so every event re-creates the control, re-fails, re-logs forever while wlsunset owns gamma (a documented-supported scenario). Add per-output `failed: bool`, gate creation on it, clear on disable. *(verified)*
10. **Fix the 4 misattached doc comments in topbar** — `pack_rows` (main.rs:258, + dead `#[allow(too_many_arguments)]`), `apply_cluster` (232), `notification_age_label` (381), all describe other functions after a reorder. Pure deletion/move; actively misleading. *(verified the pattern)*

---

## THEME 1 — Spec-parser dedup (C compositor) `[highest structural value]`

The reviewers independently flagged this from multiple angles; it's one body of work. Land items 1.1–1.3 together under one new `src/compositor/gnoblin-spec-util.{c,h}` (remember `meson.build` + the `cc` lines in `scripts/test-logic.sh` for each `*-parser-test`).

1.1 **Shared cursor tokenizer** *(high / small)* — `skip_spaces`/`at_end`/`parse_int` byte-identical in `gnoblin-rules-spec.c:10-33`, `gnoblin-actions-spec.c:11-49`, `gnoblin-output-spec.c:11-34`; `parse_whole_int` dup'd in rules:35-45 + actions:51-61. Export `gnoblin_spec_*` versions, delete the private copies. *Verified identical.*

1.2 **Shared workspace/monitor/percent parsers** *(high / small)* — `gnoblin_actions_parse_{workspace_index,monitor_index,percent}` (`actions-spec.c:85-113`) are line-for-line clones of `gnoblin_rules_parse_*` (`rules-spec.c:98-126`), only the name prefix differs. Both are tested separately. Define once as `gnoblin_spec_parse_*`; keep thin wrappers only if the two test files need the old names. *Verified.*

1.3 **Add `gnoblin_parse_nonneg_int` and fix the misleading borrow** *(medium / small)* — `gnoblin-rules.cpp:320,341,376` parse `rounding`/`border-width`/`blur-radius` by calling `gnoblin_rules_parse_monitor_index` purely for its `>= 0` check, with apologetic `/* a non-negative int */` comments. Add the honestly-named helper to the new util and call it; removes the apology comments and the "why is rounding using a monitor parser" confusion. *(Two reviewers flagged this; same fix.)*

1.4 **Shared `strtod`-whole-string parser** *(medium / small)* — the `g_strdup→g_strstrip→empty→strtod→errno/finite/trailing-garbage` scaffold is copied in `anim-spec.c:33-55`, `input-spec.c:10-31`, `output-spec.c:36-57`, `shadow-spec.c:30-46` (+ a cursor variant in `actions-spec.c:21-34`). The **trailing-garbage rule is accidentally inconsistent** (`*g_strstrip(end)!='\0'` vs `at_end(end)`). Add `gnoblin_spec_parse_double()` returning the finite value; each caller applies its own clamp/range locally. Keep `actions-spec`'s cursor `parse_double` with the int cursor helpers.

1.5 **Unify hex-colour parsing** *(medium / small)* — `parse_hex_pair` exists twice (`color-spec.c:9-23` as `guint8*`, `shadow-spec.c:14-28` as `unsigned*` — gratuitous type diff), and the `#rrggbb[aa]` length-7/9 branch is duplicated (`color-spec.c:25-50` vs `shadow-spec.c:48-72`), and `/255.0f → float[4]` is repeated in `rules.cpp:248-253` + `shadow-spec.c:67-70`. Have `gnoblin_shadow_parse_css_color` delegate its `#` branch to `gnoblin_color_parse_hex`; keep only `rgb()/rgba()` as shadow-specific. *Verified both copies exist.*

---

## THEME 2 — Rust client bootstrap dedup `[high value, removes ~5 fns × 7 binaries]`

2.1 **`apply_theme` macro** *(medium / small)* — see TOP-10 #5. The cross-type problem is **already solved** by `apply_shell_chrome_to_theme!`; the per-client `fn apply_theme` adds nothing. Also folds in `apply_shell_chrome_with` (topbar:149, dock:77).

2.2 **Promote `file_mtime` to `gnoblin_shell_ui`** *(medium / trivial)* — byte-identical one-liner-ish fn in topbar:1399, dock:447, notifyd:106. No type param needed — make it a plain `pub fn`, delete the three copies.

2.3 **Shared runtime-flag helper** *(medium / small)* — `dnd.rs:7-34`, `nightlight.rs:13-40`, `notifcenter.rs:25-52` are byte-identical (`path/is_on/set/toggle`) apart from the basename; the `XDG_RUNTIME_DIR`-join idiom recurs 8+ times (also theme.rs:53, lib.rs:1818/2464). Add `runtime.rs` with `runtime_file(name) -> Option<PathBuf>` + a `FileFlag` newtype; each module becomes a thin binding. *Verified the three is_on/set/toggle copies.* A one-line bugfix (e.g. relative `XDG_RUNTIME_DIR`) currently needs editing every copy.

2.4 **`apply_shell_motion` macro + drop dead `prefs` accessors** *(medium / small)* — fold `apply_shell_motion` (topbar:155, dock:83, notifyd:100) into a macro alongside 2.1. Separately, `prefs::motion_scale()` has **zero** callers and `prefs::animations_enabled()` is called only by `motion_scale()` itself — everyone uses `shell_motion()`; delete both (`prefs.rs:114-120, 168-176`), retarget the module header.

2.5 **`BarConfig::fullscreen_overlay()` + `FULLSCREEN_ANCHOR` const** *(low / small)* — the all-edges `Anchor::TOP.union(...)` + `Layer::Overlay` + `height:1` + `full_height:true` is spelled out identically in osd:127, power-menu:165, window-menu:211, notifyd:396, launcher:414 (and two spots use the `|` spelling: wallpaper:619, lib.rs:3268). Add `BarConfig::fullscreen_overlay(ns)` + `pub const FULLSCREEN_ANCHOR = Anchor::all();` to shell-ui.

2.6 **Reuse shared `RuntimeError`/`runtime_error`** *(low / trivial)* — redefined locally in wallpaper:45-49 and night-light:24-28 despite `gnoblin_shell_ui` exporting `pub` versions (lib.rs:60-64); both already depend on the crate. Delete locals, `use gnoblin_shell_ui::{runtime_error, RuntimeError};`.

2.7 **Build-script helper** *(low / trivial — opportunistic only)* — 7 near-identical `build.rs` differing only in the `.slint` filename. A `gnoblin_shell_ui_build::compile_ui(path)` collapses each to one line. Low value; do only if touching build anyway.

---

## THEME 3 — Bug-risk & correctness `[do early — these are behaviour, not cosmetics]`

3.1 **Config parser empty-key drift** — TOP-10 #1. `config.rs:138`.

3.2 **Inspector JSON non-finite bypass** — TOP-10 #3. `gnoblin-control.cpp:712-723`.

3.3 **Output retry budget never resets** — TOP-10 #7. `gnoblin-output.cpp:248`.

3.4 **foreign-toplevel ACTIVATED mismatch** — TOP-10 #8. `...management.c:133`.

3.5 **night-light gamma re-fail loop** — TOP-10 #9. `night-light/src/main.rs:239`.

3.6 **`--role=` empty inline value yields `Some("")` not `None`** *(low / trivial)* — `args.rs:44` `split_once('=')` on `--role=` → `Some("")`, while the space form correctly gives `None`. Map empty inline to `None` (or `value().filter(|v| !v.is_empty())`); add a test. *Verified.* Minor but a real inconsistency between two equivalent inputs.

3.7 **`get_keys` returns duplicates; doc implies a set** *(medium / small)* — `gnoblin-config.c:291` appends every entry incl. repeats, forcing `gnoblin-control.cpp:175-196` to carry a `seen` GHashTable to avoid double-grabbing a rebound accelerator. The Rust side has no `get_keys` equivalent (offers de-duplicated `entries_with_prefix`). Either de-dup inside `get_keys` (keep first-seen order) and drop control.cpp's `seen`, or document "keys may repeat". Option (a) preferred — both callers want distinct keys. *Verified only 2 callers.*

---

## THEME 4 — Larger structural dedup (medium effort, real payoff)

4.1 **Share the rounded-rect SDF GLSL** *(high / medium)* — `gnoblin-blur.cpp:75-86` (`gn_sd_circle`/`gn_sd_squircle`) and `gnoblin-rounded.cpp:70-88` (`sd_circle`/`sd_squircle`) are byte-identical bar a local rename; the squircle exponent `n = 5.0` is hardcoded in **both** (blur:83, rounded:85) and the circle→squircle blend rule is dup'd too. This is the product's signature corner shape — tuning it in one place silently diverges the other; blur.cpp:66 even comments it "mirrors gnoblin-rounded". Extract `GNOBLIN_SDF_GLSL` into `src/compositor/gnoblin-sdf-glsl.h`, prepend in both, call shared names. *Verified.*

4.2 **Promote `gnoblin_window_type_name()`** *(medium / small)* — the Mutter-type→config-string map is **triplicated**: `anim.cpp:75-111`, `shadow_window_type_name` (`shell-plugin.cpp:282-306`), inlined again in `resolve_shadow_spec` (`control.cpp:503-527`). They've **already drifted** (shadow omits tooltip/notification/desktop/dock/dnd — easy to mistake for a deliberate filter). One shared `const char* gnoblin_window_type_name(MetaWindow*)` + a tiny `menu_like` predicate. *(This also subsumes the smaller anim.cpp:128-148 / shell-plugin menu-predicate dup findings.)*

4.3 **Shared overlay grab/teardown lifecycle** *(medium / medium)* — lock/switcher/overview each build the same full-screen reactive actor + `clutter_stage_grab` + key-focus + destroy-handler scaffold but with **three inconsistent teardown strategies**: lock disconnects-before-destroy and centralizes in `free_lock` (`lock.cpp:181,213`); switcher (`switcher.cpp:283-291`) and overview (`overview.cpp:201-207`) duplicate the dismiss logic in both close + destroy and rely on idempotent re-entry. Extract `gnoblin-overlay.{h,cpp}` adopting the lock's discipline; **minimum** fix: make switcher/overview disconnect the destroy handler first so the double-run reliance goes away.

4.4 **Share `window_is_exposable` + `window_app_id`** *(medium / medium)* — byte-identical in both foreign-toplevel files (`list.c:54-76`, `management.c:64-86`); `window_app_id`'s 3-level fallback must stay in sync so both protocols advertise the same app_id. Extract to a shared overlay file (needs protocol manifest + `gen-gnoblin-protocols-patch.sh` wiring per `src/protocols/README.md` — that's why it's medium not trivial). If too heavy, at minimum cross-reference comments.

4.5 **Slint `PopoutChrome` component** *(medium / medium)* — `DatetimePopout` and `ControlCentrePopout` duplicate ~80 lines of resolved-bg/fg/border + animate + open/close `vis` state machine + scale-wrapper + tint + highlight strip + inset border (`Popouts.slint:933-949/1520-1554` etc.), and the same scaffold recurs 3× in `Compositor.slint`. Extract a `PopoutChrome`/`ShellSurfaceChrome` owning the chrome + open-close states with a content slot. *This is the natural home for the chrome-overlay dedup in notifyd/launcher/osd (Theme 7) too — do them together.* Resolve colours from `Theme` tokens inside it.

4.6 **`lib.rs` is 3422 lines — extract `desktop.rs` + `icons.rs`** *(medium / large)* — bundles 5 unrelated concerns; the actual "shared runner" (the file's stated purpose) is buried. Move XDG desktop-entry block (~242-963 + the 630-line test mod) → `desktop.rs`, icon/XPM block (~1601-2032) → `icons.rs`, re-export the public fns. No behaviour change, ~1800 lines leave the root. Sequence this **after** the small lib.rs dedups (5.x) so they move cleanly with their code.

---

## THEME 5 — Small Rust dedup inside `shell-ui/lib.rs` (trivial, fold into Theme-4.6 move)

5.1 `request_redraw()` helper — `if let Some(a)=&self.adapter { a.needs_redraw.set(true); }` at 5 sites (2601/2675/2730/2982/3051). *(low/trivial)*
5.2 `inspect_dir()` helper — `GNOBLIN_INSPECT` gate + `XDG_RUNTIME_DIR/gnoblin-inspect` create-dir duplicated (1815-1822, 2461-2468). *(low/trivial)*
5.3 `json_escape()` free fn — the `esc` closure redefined 3× in inspector fns (1830, 1902, 1947). *(low/trivial)*
5.4 `first_output_info()` — `outputs().next().and_then(info)` repeated 3× (2412, 2470, 2615). *(low/trivial)*
5.5 Single `[Desktop Entry]` scan — `desktop_exec` + `desktop_entry_dbus_activatable` each `read_to_string` + re-scan the same file back-to-back per launch (617-685). *(low/small)*

---

## THEME 6 — Stale / misattached comments (mostly trivial, high readability ROI)

**Misattached doc blocks (a function was inserted between a doc and its fn — move the block, don't reword):**
6.1 `control.cpp:556-564` "Dump the live scene as JSON" sits above `find_shaped_texture`, belongs above `build_scene_json` (621). *(medium/trivial)*
6.2 `control.cpp:309-314` GetActiveWindowMenu menu-export doc on `find_focused_window`; belongs at the handler (911). *(medium/trivial)*
6.3 topbar 4× misattached — TOP-10 #10: `pack_rows`:258 (+dead `#[allow(too_many_arguments)]`:260), `apply_cluster`:232, `notification_age_label`:381, `prettify_app` use-import doc:481.
6.4 `lib.rs:3380` `load_backdrop` doc attached to `prettify_app`; move to 3403.
6.5 `shell.rs:68-69` `activate_app` doc describes `close_app_windows` (which has none); move down to 85.
6.6 `shell.rs` n/a — `qsplugin.rs:642-644` carries `load_configs`'s providers description on `read_qs_sections` (which doesn't read providers); delete 642-644.
6.7 `gnoblin-shell.cpp:313-315` hotplug comment glued above `apply_output_once` (which has its own at 316-317); move down to `on_monitors_changed_autostart` (323).

**Contradictory / factually-wrong comments:**
6.8 Dock comment contradiction — TOP-10 #2, `rules.cpp:610-616`.
6.9 `gnoblin-shell.cpp:9-11` header names a config key `autostart = [...]` **that doesn't exist** — real keys are `[startup] exec`/`exec_per_output`. Most prominent doc in the entry-point file points at a non-existent schema. *(medium/trivial)*
6.10 `blur.cpp:496-497` "Radius halved to match half-res grid" contradicts the code 4 lines down (fixed 1.2px on a radius-scaled grid); delete the stale sentence, keep 498-500. Same class: `blur.cpp:293-296` "halving / 2×2 blocks" but factor is `radius/5`. *(medium/trivial each)*
6.11 `WindowChrome.slint:172-174` claims CSD clip radius "matches GTK's 14px" — it's **18px**, and 14px was the documented bug (Tokens.slint:287-294). *(medium/trivial)*
6.12 `Tokens.slint:387-407` overlay-motion block describes pre-config-indexed behaviour (220ms, Material 0.4/0/0.2/1, with worked arithmetic) that no longer runs — defaults are now c31 @ 250/150ms; Material is only the out-of-range fallback. Rewrite. *(medium/small)*
6.13 `quicksettings.rs:1-2` + `topbar/main.rs:232` say the snapshot feeds the "control-centre popout" — it now only feeds the topbar status cluster (popout tiles are qs-plugins). *(medium/trivial)*
6.14 `config.rs:65-67` `entries_with_prefix` doc claims "first-seen file order" but iterates a HashMap then sorts alphabetically; the `seen` HashSet is unreachable (keyed by (section,key), unique). Fix doc to "sorted by key", drop `seen`. *(medium/trivial)* *(two reviewers, same finding)*
6.15 `config/gnoblin-config.h` doc fixes: overview omits the embedded-defaults base layer (the one real C↔Rust divergence — worth a sentence so the asymmetry doesn't drift); bool-token list at :18 omits `yes/no/1/0`; `get_keys` doc at :48-50 cites `[snap]` as a consumer but `[snap]` uses `get_string` (real second consumer is `[output]`). *(medium+low/trivial)*

**Lower-value stale comments (batch when nearby):** `launcher/main.rs:64` ("Run will be added" — it exists), `launcher/main.rs:82` (`accessory` "unused" — it's rendered), `rounded.h:17-19` (omits RING style), `shadow.h:8` (50% at window vs spread edge), `blur.cpp:38-60` ("normalised Gaussian" sums to 0.90 — decide: normalise or relabel), `anim.h:16-18` (maximize example contradicts ease-out defaults), `lock.cpp:342` ("clock-ish prompt" — no clock), `overview.cpp:205` (opaque-backdrop only, ignores blur path), `prefs.rs:51-53` (stale `#[allow(dead_code)]` on used CURVE_POP), `Dock.slint:1-10` + `ContextMenu.slint:4-7` headers cite stale numbers, `IconButton.slint:2` ("dock icons" — not used by dock).

---

## THEME 7 — Slint tokens / dead UI props (trivial deletions)

7.1 **`ControlCentrePopout` 6 dead colour props** *(medium/trivial)* — `resolved-tile-bg`, `-hover`, `accent`, `icon-on-active`, `icon-bg-inactive`, `footer-divider` (Popouts.slint:1532-1549) have zero reads (tiles read `ShellPalette` directly); 3 are verbatim `ShellPalette` dupes; the animate blocks at 1555-1557 animate nothing. Delete props + animates + stale comment 1534-1538.
7.2 **`DatetimePopout.clock-text` dead** *(medium/trivial)* — declared (890) + fed from `Compositor.slint:821` but never rendered. Remove both (or render it); update header "time" claim at :2.
7.3 **Dock backdrop props never rendered** *(medium/small)* — `backdrop`/`-screen-w/h`/`-offset-y` (Dock.slint:304-307) fed from `Compositor.slint:478-481` but `pill-bg` only paints the tint; no `Image` slice. Comments at 336-339/368 falsely promise "backdrop+tint". Delete props + assignments (matches current compositor-frost behaviour).
7.4 **Tokens with zero consumers** *(low/trivial)* — `Theme.highlight-*-top` ×4 (223-226) + 2 re-exports (372-373), `motion-overlay-curve` alias + its "callers haven't migrated" comment (413-414, factually wrong), `panel-corner-radius` (315), `motion-overlay-duration` (410). Delete.
7.5 **`Panel.icon-bluetooth` dead** (481), **StatusIcon imports `Theme` unused** (5), **IconButton imports `Tokens/Theme` unused** (4). *(low/trivial)*
7.6 **Two "accent blues"** *(low/small)* — `rgba(96,165,250)` (today-chip Popouts:1209, snap-preview Compositor:798-799) diverges from `ShellPalette.accent` `rgba(53,132,228)` with no token; a retheme misses them. Add `Theme.accent-highlight` token.
7.7 **Hardcoded menu-row text colours** *(low/small)* — `ContextMenu.slint:287-306` uses literal rgba bypassing tokens; `ShellPalette.icon-on-active` already exists for the active case. Add `text-on-accent`/`text-disabled`.
7.8 **Notification card token mismatch** *(medium/trivial)* — `notifications.slint:56-57` uses `menu-shadow-*` while body uses `popout-corner-radius`; siblings launcher/osd use `popout-shadow-*`. Switch to popout shadow tokens.
7.9 **Re-indent outdented `body :=` blocks** *(low/trivial)* — `notifications.slint:60-147` + `osd.slint:38-101` children sit flush with their parent; no Slint formatter in-repo so it's hand-maintained. Mechanical re-indent.

---

## THEME 8 — Dead code in C compositor / protocols (trivial deletions)

8.1 `quicksettings.rs` gutting — TOP-10 #6 (the headline dead-code item; also has runtime cost).
8.2 **layer-shell write-only serial fields** *(medium/trivial)* — `configure_serial` + `acked_configure_serial` (`meta-wayland-layer-shell.c:98-99`) only ever written; real bookkeeping is the `configure_serials` GQueue + `has_acked_configure`. Delete fields + 3 assignments.
8.3 **layer-shell redundant `window->type` set** *(medium/trivial)* — :1254-1257 sets type, then :1265 `apply_window_type_and_layer` re-derives it identically from unchanged state. Delete 1254-1257.
8.4 **`gnoblin_rules_blur()` dead write-only chain** *(medium/small)* — accessor (rules.cpp:701), the `blur` field (32, "legacy"), and both `hints->blur=` writes (371,382) are all dead; live path uses `blur_set/blur_on`. Remove all. *(Keep `no_shadow`/`opacity` — they have callers.)*
8.5 **`anim.cpp:145-152` unreachable menu close-defaults** *(medium/trivial)* — menu-family `close` defaults can never fire (gated off by `wants_window_close_animation`); header even says so. Delete the branch + the doc example.
8.6 **Dead `is_open()` accessors** *(medium/trivial)* — `gnoblin_switcher_is_open`/`gnoblin_overview_is_open` (switcher.h:36/overview.h:31, defined .cpp:49/38) have zero callers. Delete both decls+defs.
8.7 **notifcenter legacy flag** *(medium/trivial)* — `toggle()` (notifcenter.rs:48) fully dead, `set(true)` unreachable (only `set(false)` callers exist); notifyd's per-tick `is_open()`+`set(false)` probe (main.rs:366-370) is inert. Delete `toggle()`; collapse to a `clear_legacy_center_flag()`.
8.8 **Smaller dead state** *(low/trivial, batch):* `control.cpp:52,982` `accelerator_handler` write-only; `shadow.cpp:146-147` always-overwritten `pad=48/radius=12`; output-power/gamma context signal-ids never disconnected (output-power:43-44, gamma:50 — note: overlay source copied into `subprojects/mutter`); `maybe_add_blur`'s unused `plugin` param (shell-plugin.cpp:613-640); `switcher.cpp:21` stray `gnoblin-anim.h` include; layer-shell `focus_exclusive_layer_surface` called in both apply+post_apply (1018+1077); `provider.rs:30` speculative `id` field; launcher `kind:"provider"/"web"` set but view only branches on `"calc"`.

---

## Smaller naming / clarity / consistency (do opportunistically)

- **`border_style_name` magic ints** — `control.cpp:541-557` switches `0..3` instead of the in-scope `GnoblinBorderStyle` enum; drop the `(int)` cast at :770. *(low/trivial — genuinely improves coupling.)*
- **`find_focused_window` reinvents `meta_display_get_focus_window`** — `control.cpp:315-331` hand-rolls a list walk that 4 other sites do via the builtin; delete it, call the builtin at :915. *(medium/trivial)*
- **Extract `get_actor_rounded()` + `GNOBLIN_ROUNDED_EFFECT_NAME`** — `"gnoblin-rounded"` open-coded 6× (shell-plugin.cpp:573,596,772,829,1219,1237) while blur got a constant+accessor; a typo silently disables rounding. *(low/trivial)*
- **`GNOBLIN_SHADOW_WINDOW_ACTOR_KEY` constant** — magic qdata string set at shell-plugin.cpp:724, read 1000 lines later at :1736; sibling pointer uses a constant. *(low/trivial)*
- **`json_bool()` helper** — `? "true":"false"` ×19 in the scene dumper (control.cpp); file already has json_str/color/fvec. *(low/small)*
- **`MenuAddr::is_dbusmenu` accepts undocumented `"kde"`** — compositor only emits `gtk`/`dbusmenu` (control.cpp:938/950); drop the alias or document it (appmenu.rs:33). *(low/trivial)*
- **`actions-spec.h:16` param `value` vs def `out`**, **`make_copy_pipeline`→`make_downsample_pipeline`** (blur.cpp:297, name lies — it downsamples), **`gnoblin_actions_parse_uint` header param** — pure rename consistency. *(low/trivial)*
- **`appmenu.rs:284-318` dbusmenu proxy built 3× identically** → `dbusmenu_proxy()` helper; **`shell.rs:55-99` `Connection::session()+ShellProxy::new()` ×4** → one helper. *(low/small — fold together.)*
- **`theme.rs:61-71` double `Config::load()`** (is_dark + shell_chrome parse the file twice, split-read race) → `*_with(cfg)` variants. *(low/small)*
- **topbar `config_i32`/`parse_i32` px-suffix parsing** — promote to `Config::get_i32` when next touched (not yet duplicated). *(low/small)*

---

## Leave alone / not worth it

- **`action_names[]` ↔ dispatch parallel-list guard** (`gnoblin-actions.cpp:348-507`) — they currently match (26=26, verified); the proposed `#ifndef G_DISABLE_ASSERT` self-check is more machinery than the risk warrants. A one-line cross-reference comment is the *most* I'd do, and even that is optional.
- **`control-center/cc-gnoblin-panel.c` protocol-toggle split** (only 2 of 9 toggles, different key names, no GSettings→gnoblin.conf bridge) — real, but it's a **product/architecture decision** (which is the source of truth?), not a sweep cleanup. Flag for a separate discussion; don't fix blind.
- **`settings_changed_cb` rebuilds all monitor rows** (cc-gnoblin-panel.c:345) — a real inefficiency, but the panel is a rarely-open GTK settings dialog; the key-filter refactor is medium effort for negligible user impact. Defer.
- **`gnoblin_config_get_string` autofree+steal "dance"** (config.c:242), **`clean_value` re-strips** (config.c:74), **`config.rs` double-parse in theme path** — micro-tidies; only touch if already editing the function. The g_autofree/g_steal is harmless idiom, not a bug.
- **`gnoblin_output_parse_spec` lenient (ignores malformed tokens)** — likely intentional best-effort; just add a one-line "intentionally partial" comment rather than wiring error handling.
- **`menu_like_shadow_type` ⊂ `shadow_window_type_name`** (shell-plugin.cpp:282-318) and **`power-menu`↔`window-menu` Slint near-twins** — defensible as-is; the cross-reference comment / `MenuOverlay` extraction is low priority since they're independent. Subsumed anyway if you do Theme-4.2 / 4.5.
- **`lock.cpp:162,243` redundant `the_lock &&`** and **`Provider.id` YAGNI field** — cosmetic; fine to leave.
- **`prefs.rs` keeping `animations_enabled()` as public API** — if external-facing, just `#[allow(dead_code)]` it; otherwise delete (covered in 2.4).

---

## Overall read on codebase health

**Healthy and well-tended.** The volume here is mostly *consolidation debt from rapid iteration*, not rot: the recurring pattern is "a good helper/macro already exists (`apply_shell_chrome_to_theme!`, `json_fvec`, `effects_color`, ShellPalette tokens) and a sibling site hand-rolls the same thing instead." That's the cheapest possible class of finding to fix and the easiest to keep fixed. The genuinely-important items are few and concentrated: **the C↔Rust config-parser drift (empty keys) and the inspector JSON non-finite bypass are the only two I'd call near-bugs**, plus three reachable-but-degraded paths (output retries, night-light gamma loop, foreign-toplevel ACTIVATED). The spec-parser tokenizer triplication and the SDF-GLSL duplication are the two structural items most worth a dedicated session because they sit on code that *will* be edited (parsers, the signature corner shape) and silently diverge. Comment staleness is widespread but almost entirely from function-reorder drift — bulk-fixable in an afternoon and worth doing because several actively point readers at non-existent config keys / deleted behaviour.

Files most worth your attention, in order: `src/clients/shell-ui/src/config.rs` + `src/config/gnoblin-config.c` (keep these two in lockstep), `src/compositor/gnoblin-spec-util.{c,h}` (new — the dedup hub), `src/compositor/gnoblin-control.cpp` (JSON safety + several stale docs), and `src/clients/shell-ui/src/lib.rs` (the 3422-line outlier — the extraction unblocks the small dedups).