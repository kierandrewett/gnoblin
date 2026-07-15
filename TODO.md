# gnoblin — TODO

Working tracker for the **patched-GNOME** direction (pinned Mutter 49.5 +
GNOME Shell 49.6). Kept in the repo, not Claude's task tool. Newest asks bubble
to the top of **Next**.

> The from-scratch C++ compositor + Rust/Slint clients were retired. Recover
> them from the `archive/cpp-compositor` tag.

## Current remediation

The July 2026 repository audit is being fixed in ordered, working increments.
Security and verification gates land before runtime, packaging, and documentation
cleanup. Checked items are committed and independently verified.

### Immediate safety

- [x] Move shared `/tmp` executables, generator files, and persistent logs into
  private state.
- [ ] Stage Mutter RPM `Source1` from a clean RPM build directory.

### Verification baseline

- [ ] Fail shell smoke tests on fatal runtime diagnostics.
- [x] Exercise stock GNOME and Gnoblin session isolation separately.
- [ ] Add request, invalid-input, lifecycle, and disconnect protocol clients.
- [ ] Replace fixed asynchronous sleeps with bounded state waits.
- [ ] Add honest unit, built-headless, and real-host verification aggregates.

### Session security

- [ ] Remove unconditional portal Access approval.
- [ ] Make shell unsafe mode an explicit, isolated development opt-in.
- [ ] Register privileged Mutter protocols only for Gnoblin.
- [x] Scope native-topbar, extension-validation, and notification changes to
  Gnoblin.
- [ ] Replace marker-file portal grants with trustworthy requester identity and
  authorisation.

### Protocol correctness

- [ ] Use checked arithmetic for screencopy geometry and byte sizes.
- [ ] Honour the foreign-toplevel manager destructor contract safely.
- [ ] Use checked arithmetic for layer-shell geometry and exclusive-zone struts.

### Runtime consistency

- [ ] Reconcile direct GSettings changes into live feature state.
- [ ] Synchronise Settings switches and grants with D-Bus completion/signals.
- [ ] Report reload success only after asynchronous reload work completes.

### Tooling and packaging

- [ ] Refuse destructive patch/reset operations when submodules contain
  unexpected work.
- [ ] Centralise runtime test environment setup.
- [ ] Support one configurable prefix and library-directory contract.
- [ ] Publish deterministic source archives atomically.

### Final cleanup

- [ ] Correct capability, source-map, configuration, and support documentation.
- [ ] Reduce this file's stale historical `Done` detail after the fixes land.
- [ ] Run focused, full headless, and applicable real-host verification.
- [ ] Run structural, security, simplification, and similarity review.

## Done

- **Build revival** — `just dev` builds patched mutter + gnome-shell + session
  data into `./install`. `just gnome-verify` (boots gnome-shell in gnoblin mode,
  checks `zwlr_layer_shell_v1`), `just gnome-dbus-verify` (org.gnoblin.*
  round-trip), `just test-config`, `just test-mutter`.
- **Session-mode UI strip** — `src/data/session/modes/gnoblin.json`
  (parentMode `user`, empty panel, `hasOverview:false`, minimal components).
  Stock chrome gone without heavy JS patching.
- **`org.gnoblin.Shell` control component** — `gnoblinControl.js` session
  component: Ping / GetVersion / Reload + ListFeatures/GetFeature/SetFeature/
  FeatureChanged, persisted in the `org.gnoblin.shell` `disabled-features`
  gschema key. Feature toggles now cover `osd` (+ per-type), `screenshot`,
  and `notifications`.
- **Wayland soft-reload** — reloads theme + extensions in-process so windows
  survive; over D-Bus (`Reload`) and via `Alt+F2` `r`.
- **Extension hot-reload + GJS scripting layer** — `ReloadExtension` cache-busts
  an extension's import; `ScriptHost` loads/hot-reloads
  `~/.config/gnoblin/scripts/*.js` against a small event bus. Both driven over
  `org.gnoblin.Shell` / `gnoblinctl`, both covered by `gnome-hot-reload-verify`
  and `gnome-scripting-verify`.
- **GNOME Shell patch set** — Gnoblin-only relaxed extension loading, native
  top-bar suppression, and notification ownership toggles; correct portal
  Access request cancellation; `--disable-extensions` flag; branding; feature
  schema. The stock `user` mode retains upstream panel, extension-validation,
  and notification ownership behaviour.
- **Shell privilege policy** — `org.gnome.Shell.Eval` keeps its upstream guard
  everywhere except an isolated devkit explicitly started with
  `GNOME_DEVKIT_UNSAFE_MODE=1`.
- **Mutter protocol overlays** — layer-shell v5 + the rest, each gated via
  `gnoblin.conf` `[protocols]`.
- **Unattended screensharing** — `xdg-desktop-portal-gnome` patch adds
  macOS-style per-app persistent screencast/RemoteDesktop grants (tick "always
  allow" once, never re-prompted), stored under
  `~/.config/gnoblin/portal-grants/`, managed via `gnoblinctl screen-grants` /
  `revoke-grant`.
- **`gnome-control-center` fork for gnoblin settings** — a `gnoblin` panel
  driving feature toggles, screencast grants, and a reload button; hides the
  Multitasking panel. `just dev-settings` (not part of `just dev`).
- **Devkit** — `just gnome-devkit` boots a visible nested gnoblin session +
  wired terminal for iterating on your own chrome without logging out;
  `just gnome-devkit-verify` regression-tests the env plumbing.
- **Retirement cleanup** — deleted `src/compositor/`, `src/clients/`,
  `subprojects/slint`, `dist/`, and the old C++-compositor-era devkit
  scripts + tests (the current `Devkit` entry above is a from-scratch
  replacement for the patched-GNOME stack, not a survivor of this cleanup).
- **Docs + dead-code sweep** — user guides under `docs/` (installation, devkit,
  testing, configuration), root README now links into them. Removed docs/
  scripts/schema left over from the pre-pivot layout (stale gschema keys with
  no reader, dangling references to deleted scripts, a missing
  `scripts/devkit-document-portal-stub.py` the devkit's isolated D-Bus config
  depended on but never shipped).
- **Real-login session registration** — `just dev-session` now also installs
  gnoblin-specific systemd --user units (`org.gnoblin.Shell.target` /
  `@wayland.service`, pointed at the patched gnome-shell via a wrapper that
  prepends this prefix's lookup paths); `just dev-session-register` links
  them into your live systemd --user instance and prints the one remaining
  (root) command to add "Gnoblin" to your login manager's picker. Previously
  a prefix-only `just dev-session` install had no working path to a real
  login at all: gnome-session's `RequiredComponents=org.gnome.Shell` would
  have resolved to a system GNOME Shell install's own systemd unit, not the
  patched one, silently launching the wrong shell. Gnoblin-specific unit
  names sidestep that instead of shadowing the shared `org.gnome.Shell`
  unit, so a system GNOME session (if present) is unaffected either way.
  `systemctl --user link` + `daemon-reload` was verified on a real machine
  with an existing system GNOME install: the units link cleanly, and
  `org.gnome.Shell@wayland.service` stays resolved to the system shell
  throughout (no shadowing). Actually selecting "Gnoblin" at GDM and
  logging in is NOT yet verified — that needs the manual root step in
  docs/installation.md, then a real logout/login, which wasn't done as part
  of this pass.
- **RPM packaging** — `packaging/rpm/gnome-shell.spec` now builds a
  `gnoblin-session` subpackage (session mode, login entry, gnoblin-specific
  systemd units, `gnoblinctl`/`gnoblin-session`/`gnoblin-shell-service`),
  `Requires:`-pinned to the exact matching `gnome-shell` build.
  `rpm_projects` in the Justfile now covers both `mutter` and `gnome-shell`.
  `just rpm mutter` / `just rpm gnome-shell` both build clean; verified RPM
  contents (`rpm -qlp`/`rpm -qRp`) and payload (Exec=/ExecStart= correctly
  resolve to `/usr/bin/...`) but did NOT run the actual `dnf install` —
  installing replaces the system's Mutter/GNOME Shell, a real change left
  for Kieran to trigger deliberately (command in docs/installation.md).
  deb/arch scaffolds updated to describe the same package split, still not
  implemented.
- **Second dead-code sweep** — ran 4 parallel audits (patches/, protocols +
  control-center, packaging/, scripts + root config) after the first pass.
  Fixed: `.cargo/config.toml` (retired Rust leftover), stale `.gitignore`
  comments/entries, 3 patch commit-message path/reference fixes (diff hunks
  untouched, `just patch mutter` re-verified), a stale "KDE appmenu" doc
  claim, a stale "GTK4/Rust lock client" reference, stale "Phase-2/3"
  comments in `gnoblinControl.js` (features have shipped for a while), a
  stale cross-reference to the deleted Rust config parser. Extracted
  `src/tools/gnoblin-env.sh` — a 4th copy of the same prefix env-setup block
  had accumulated across `run-gnome-shell.sh`/`run-gnome-devkit.sh`/
  `gnoblin-session`/`gnoblin-shell-service`; now one shared function.
- **Third dead-code sweep (`src/`, `scripts/`, `docs/`)** — focused pass after
  Kieran flagged `src/` specifically. Removed `src/data/plugins/` (9
  `gnoblin-qs-*` scripts, 201 lines): a JSON tile/menu quick-settings
  protocol for the retired Slint top bar, self-documented as unconsumed and
  containing stale references to specific retired Rust source files by
  name. Fixed the resulting dangling references in `src/README.md`,
  `src/data/README.md`, and the root README's layout tree. Checked and
  ruled out as NOT dead: `session-lock/`/`output-management/` (legitimate
  deferred work, not leftover cruft), the protocol aggregator wiring, the
  control-center/gnome-shell-overlay manifests, and every script in
  `scripts/` (5 looked orphaned by a naive Justfile grep but are all called
  indirectly — `copy-overlay.sh` by `apply-patches.sh`, `devkit_dbus.py` by
  the 5 headless `gnome-*-verify` scripts plus `run-gnome-shell.sh`/
  `run-gnome-devkit.sh` (not `test-config.sh`, which needs no display),
  `devkit-document-portal-stub.py` by `devkit_dbus.py`,
  `wl-globals.c` compiled on demand, `gen-gnoblin-protocols-patch.sh`
  documented as the maintainer path for adding protocols). Full read-through
  of all 6 files under `docs/` cross-checked against source (feature-id
  table vs `gnoblinControl.js`, devkit env vars vs `run-gnome-devkit.sh`,
  the testing table vs the Justfile) found no drift. `gnoblin_config_get_keys()`
  had zero call sites anywhere (production or test) — kept it (the only
  primitive that can enumerate unknown key names in a section, needed for
  `[bind]`) and added the missing test coverage instead of deleting it.
- **Fourth patch-surface sweep** — removed two now-redundant GNOME Shell
  overview patches. `gnoblin` already declares `hasOverview:false` in
  `src/data/session/modes/gnoblin.json`, and the session overlay removes the
  stock panel, so `patches/gnome-shell/20-no-overview` and
  `21-workspaces-indicator` no longer need to modify GNOME Shell's stock
  `user` session or Activities button. Verified with `just patch gnome-shell`,
  `just dev`, and `just gnome-verify`.
- **License and stale tooling wording** — added a root `COPYING` file that
  scopes GPL-2.0-or-later to gnoblin's own source files and leaves pinned
  GNOME submodules under their upstream `COPYING` files. Also replaced the
  stale `screen-mirror` references in the Mutter key-event and GNOME Shell
  unsafe-mode tooling patch descriptions with generic inspection/automation
  wording.
- **Overlay key scoping** — retired `patches/mutter/20-no-overlay-key` instead
  of changing Mutter's schema default for every session. Gnoblin now installs
  `00_org.gnoblin.mutter.gschema.override` into the active schema directory and
  recompiles schemas in `install-session.sh`; because all shell launch paths
  set `XDG_CURRENT_DESKTOP=GNOME:Gnoblin`, `org.gnome.mutter overlay-key`
  defaults to `''` only inside Gnoblin and stays `Super` for the stock GNOME
  desktop default.

## Next

- Remaining feature toggle: `polkit`.
- Extension sideload ergonomics — hot-reload itself works
  (`gnome-hot-reload-verify`); sideloading is still "drop it in
  `~/.local/share/gnome-shell/extensions/<uuid>/` by hand".
- `kobel` — Kieran's personal Quickshell chrome config, in a separate repo.
- Actually select "Gnoblin" at GDM on a real machine and log in — the
  systemd/login-manager wiring is built and unit-tested, but nobody has
  done the real logout/login yet.
- Run the built RPMs' `dnf install` for real (system-package-replacing,
  deliberately not run as part of this pass — command in
  docs/installation.md).
- **Architecture questions the sweep surfaced, not acted on (your call):**
  - `layer-shell` and `screencopy` are wired directly via `patches/mutter/
    30-layer-shell/` and `30-screencopy/`, not through the
    `aggregator/meta-gnoblin-protocols.c` pattern the other 6 protocols use
    (`40-gnoblin-protocols`). Works fine as-is (no double-registration); just
    architecturally inconsistent. Migrating them means regenerating two
    patches and retesting — real work, not a quick fix.
  - `tests/config-test.c` doesn't exercise every rule in
    `src/config/README.md`'s Grammar Contract (leading/trailing trim,
    section-name trim, single-quoted values, a few others) — a coverage
    gap, not evidence the parser's wrong.
  - `foreign-toplevel-list`/`foreign-toplevel-management` share real
    duplicated logic (`window_is_exposable`, `window_app_id` fallback,
    bind/enumerate loops) — a safe extraction if you want to reduce drift
    risk, not urgent.
