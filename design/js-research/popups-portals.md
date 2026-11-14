# Built-in popups, dialogs and portal UI in the JS/TS world

Tree: `/home/kieran/dev/gnoblin-rust-wt` (`WT/`). Spec: `/home/kieran/dev/lanes/js-spec/SPECIFICATION.md` (`§n`). Read in depth: `WT/design/cutover-survey.md` 16.1(c) and 16.6, `WT/design/node-core.md` (lock, restart, F1), `WT/docs/guides/replace-built-in-dialogs.md`, `WT/data/lua/gnoblin/shell/{prompts,lock,alerts,menus}.lua`, `WT/crates/gnoblin-compositor/src/ipc/portal.rs`, `.../portal/ui.rs`, `WT/crates/gnoblin-model/src/slot.rs`, `WT/crates/xdg-desktop-portal-gnoblin/src/{lib,consent,iface/capture}.rs`. Skimmed or grepped: the other crates in the assignment. Anything not opened is marked _unverified_. API names below are **proposed**.

## 1. What the tree does today (facts the design rests on)

1. **There are two drawers for the consent dialog.** The Rust egui dialog (`gnoblin-portal-consent::ConsentUi`, run by `gnoblin-compositor/src/portal/ui.rs` `PortalUi`, shown as a `Modal` overlay by `ipc/portal.rs` `PortalService`) is the default and the fallback (owner decision O3). The stock Lua shell then claims the `portal.ui` and `prompt.secret` slots and draws its own copies (`shell/prompts.lua`, 427 lines), so on a stock session the Lua dialog is what the user sees.
2. **The compositor already enforces the trust rules on any claimant** (`ipc/portal.rs`): only the exact bytes `allow` / `deny` count; an `allow` inside `ARMING_MS = 500` is a refusal; identity (`subject`) comes from the portal backend's verified bus caller; a claimant that declines, answers garbage, leaves, or misses its deadline hands the request to the built-in dialog (Deny focused); the claimant gets half of the requester's timeout (`routable`); a locked session refuses with `locked`; only the owner's connection can claim (`ipc/privileged.rs`, SO_PEERCRED). Grants are the compositor's record (`gnoblin-authority`).
3. **Slots** (`gnoblin-model/src/slot.rs` `CapabilitySlot`): `AuthPolkit`, `PromptSecret`, `LocationAgent`, `IdlePolicy`, `SecretDerive` (services only), `PortalUi`. Only `PortalUi` has a built-in dialog. `PromptSecret` has none: with no claimant the request is refused at once and the keyring stays locked (`docs/shell-api/passphrase-prompts.md`). `AuthPolkit` and `LocationAgent` default to the services role, which draws nothing; the stock Lua shell claims neither (`grep` in `WT/data/lua`: no match), and `docs/shell-api/authentication.md` says "Not usable".
4. **There is no screencast picker today.** The app names the source in `SelectSources` (`iface/capture.rs`: `SOURCE_MONITOR`, `SOURCE_WINDOW`, `SOURCE_VIRTUAL` = a View); the user is asked only Allow/Deny for that source.
5. **The file and app chooser UIs exist but are not wired.** `gnoblin-portal-filechooser::FileChooserUi` and `gnoblin-portal-appchooser::AppChooserUi` (egui, models in `gnoblin-portal-core/src/model/dialog.rs`, answers validated in `gnoblin-compositor/src/portal/validate.rs`) are unreachable: `ipc/portal.rs` settles them `unsupported-ui`, the backend does not implement the interfaces, and `data/gnoblin-portals.conf` routes them to `gtk` or `kde` (`xdg-desktop-portal-gnoblin/src/lib.rs` header).
6. **A runtime crash deletes every shell-drawn surface.** The compositor destroys every node the old shell created in the next frame (`node-core.md` 5, "Restart"; tests `a_detached_shells_nodes_are_destroyed_with_its_connection`). Windows stay and a minimal Rust fallback places them (F1, `fallback.rs`). The `Modal` overlay is not a tree node, so the built-in dialog survives a runtime crash. The guardian restarts the runtime (D1).
7. **The lock is half Rust.** Cover, input gating, capture embargo and the PAM check (`gnoblin-lock`, `gnoblin-lock-auth`, `Lock.authenticate`, password a `SecretText`) are Rust. The password entry is `shell/lock.lua` only. A locker crash leaves the session locked and "nothing on the tree can unlock it" (`node-core.md` 2.4). With no runtime and no external `ext-session-lock` client the user sees a black cover with nothing to type into. This is the largest gap in a JS-only world.
8. **The passphrase passes through the script.** `prompts.lua` and `lock.lua` use an `Entry{secret=true}` fed by key events; the text sits in a Lua string, which cannot be wiped (`prompts.lua` header says so). A JS string has the same property.
9. **Theme today is only light or dark.** `PortalService::configure(&Config)` maps `appearance.color-scheme` to `DialogScheme::{Light,Dark}` and `DialogShell::new` uses egui's stock `Visuals::dark()/light()` (`gnoblin-portal-ui/src/ui.rs:26-43,93-105`). No accent, font, radius or button layout exists.
10. **Not found in the Rust tree:** a notification daemon (no `org.freedesktop.Notifications` in `crates/**/*.rs`), an OSD, a power menu or logout confirm (only `session.logout`, `gnoblin.session.poweroff|reboot|suspend` in `gnoblin-services/src/power.rs`), gesture hints, a stock toast beyond `shell/panel.lua` `toast()`. `docs/bring-your-own-shell.md` mentions native notifications, a keyboard-layout popup and a "recovery panel"; I found no code for them (_unverified_, the page may predate the Rust compositor).

## 2. Owner categories

- **A** Rust draws by default, always, with no runtime. JS may restyle through settings, observe, and apply policy (e.g. auto-deny). Replacement needs an explicit capability (section 4).
- **B** JS draws by default (stock TS library). A Rust fallback exists, so the surface works with the runtime down or hot-reloading.
- **C** JS only. Without the runtime the surface is absent and nothing is lost that cannot wait for the guardian's restart.

## 3. Surface table

"Needs no runtime" = must the surface work when the JS runtime crashed or never started. "Script spoof" = must the compositor stop a script from forging it.

| #   | Surface                                                                                                                              | Today's drawer                                                                                                         | Needs no runtime                                                          | Script spoof / secret entry                                                                                                                                                                       | Owner                                                                           | Reason                                                                                                                                                                           |
| --- | ------------------------------------------------------------------------------------------------------------------------------------ | ---------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 1   | Portal consent: Screenshot, RemoteDesktop, InputCapture, Secret, Access (a11y consent), Clipboard, GlobalShortcuts, ScreenCast Allow | Rust egui (`gnoblin-portal-consent`) + stock Lua `prompts.lua` over `portal.ui`                                        | Yes: it gates capture and input injection (D14, O3)                       | Yes: Deny default, 500 ms arm, identity and grants already compositor-enforced (fact 2)                                                                                                           | **A**                                                                           | One implementation, always there, unspoofable look. The rules already live in Rust, so a JS drawer adds only appearance, which theming gives. Do not port `prompts.lua` (see Q1) |
| 2   | Screencast / window picker                                                                                                           | none: app names the source                                                                                             | No: fallback is consent for the app-named or primary source               | Selection must be a subset of what Rust offered; Allow carries the row 1 rules                                                                                                                    | **B** (new)                                                                     | Only the scene can draw live thumbnails (a `View` per source). Rust fallback = row 1 consent for one default source                                                              |
| 3   | File chooser                                                                                                                         | Rust egui, built but unwired; gtk/kde portal in practice                                                               | Not needed: apps get the gtk portal                                       | Answer validated by `validate.rs` (paths, filter)                                                                                                                                                 | **A** (when wired)                                                              | Needs directory listings and overwrite logic the JS sandbox has no filesystem for (spec §3). JS restyles only                                                                    |
| 4   | App chooser                                                                                                                          | Rust egui, built but unwired                                                                                           | No                                                                        | Choice must be in the offered list (`validate.rs`)                                                                                                                                                | **B**                                                                           | Pick from a list given in the request: no privilege. Shell icons and grid fit better than egui                                                                                   |
| 5   | Secret / passphrase prompt (`prompt.secret`, keyring unlock)                                                                         | stock Lua `prompts.lua` only; **no Rust built-in**, refused without claimant                                           | **Yes**: today a dead runtime means the keyring cannot be unlocked        | Secret entry: yes. Phishing by a script drawing a fake prompt is the threat                                                                                                                       | **A** (Rust built-in is new work)                                               | Fail-closed is correct but too harsh as the only path. JS may restyle; replacement needs the secure-entry capability (4.3)                                                       |
| 6   | Polkit authentication (`auth.polkit`)                                                                                                | none; polkit agent registers, no UI, "Not usable"                                                                      | Yes                                                                       | Secret entry: yes. The agent crate says the password never reaches it (`gnoblin-service-polkit/src/lib.rs`) and replies on a provider's `allow`: how polkitd's helper is run is _unverified_ (Q6) | **A** (new)                                                                     | A script must never decide whether a password was right. Rust collects and verifies; JS restyles and may add context text                                                        |
| 7   | Location consent (`location.agent`, GeoClue `AuthorizeApp`)                                                                          | none                                                                                                                   | No                                                                        | Level capped by Rust policy (`Policy::DENY` default)                                                                                                                                              | **A**                                                                           | A consent question. Reuse the row 1 screen with different text; script may intercept with a policy                                                                               |
| 8   | GnuPG pinentry (`pinentry-gnome3` -> `gcr-prompter`)                                                                                 | protocol client: an ordinary toplevel, not Gnoblin's (`tests/pinentry.rs`)                                             | n/a                                                                       | Cannot be made unspoofable: it is a normal window                                                                                                                                                 | not ours                                                                        | Document only. JS may style it with a client rule; it is not a trusted surface                                                                                                   |
| 9   | Session lock screen                                                                                                                  | `shell/lock.lua` (entry, clock); Rust owns cover, input gate, PAM                                                      | **Yes**: a crash or a broken stock library leaves a black cover (fact 7)  | Secret entry: yes. Cover and unlock path already Rust; unlock only via `Lock.authenticate` or the owner's `ext-session-lock` client                                                               | **B**, Rust fallback is new work                                                | JS draws the pretty screen (clock, blur, notifications). Rust fallback: password entry plus clock on the cover, calling the same PAM check                                       |
| 10  | Window menu                                                                                                                          | `shell/menus.lua` (606 lines); a missing shell leaves requests unanswered                                              | No                                                                        | No secret; a menu is not a trust surface                                                                                                                                                          | **C**                                                                           | Pure policy and looks over `Window` requests. Disable is a setting                                                                                                               |
| 11  | Title bars / decorations                                                                                                             | stock `Titlebar` (`decorations.lua`, `Canvas` painted by `gnoblin-paint`); `xdg-decoration` answers client-side always | Partly: an SSD-less window has no drag or close while the runtime is down | No                                                                                                                                                                                                | **C** (Q4: fallback titlebar?)                                                  | Decision 2a (`node-api-refactor.md:1892`): Gnoblin draws them as shell nodes. Style belongs to JS (`client.style`, spec §6)                                                      |
| 12  | Alerts: "not responding" (Wait / Force quit), `toast()`                                                                              | `alerts.lua`, `panel.lua`                                                                                              | No                                                                        | Force quit calls `Window:kill`; Rust refuses the compositor's own pid and init                                                                                                                    | **C**                                                                           | Convenience. Force quit is guarded in Rust; a spoofed alert has no more power than a script calling `kill`                                                                       |
| 13  | Notifications                                                                                                                        | none in Rust (fact 10); external daemon (mako, Bingux)                                                                 | No                                                                        | Not trusted                                                                                                                                                                                       | **C**                                                                           | A JS notification centre or an external client. No compositor role                                                                                                               |
| 14  | Power menu / logout confirm                                                                                                          | none; `session.logout`, `session.poweroff` are owner IPC                                                               | No                                                                        | No secret; destructive but owner-only calls                                                                                                                                                       | **C**                                                                           | A plain JS menu over existing owner calls. Rust keeps `LOGOUT_EXIT_CODE` handling                                                                                                |
| 15  | Overview                                                                                                                             | `overview.lua` (9 lines: opacity)                                                                                      | No                                                                        | No                                                                                                                                                                                                | **C**                                                                           | Policy over workspace nodes and Views                                                                                                                                            |
| 16  | Launcher                                                                                                                             | `launcher.lua`: an `Entry` that calls `spawn`                                                                          | No                                                                        | No secret                                                                                                                                                                                         | **C**                                                                           | A script's own UI                                                                                                                                                                |
| 17  | OSD (volume, brightness, keyboard layout)                                                                                            | none (pad OSD was removed, `gnome-shell-retirement.md:71`; compositor emits `pad-help-requested`)                      | No                                                                        | No                                                                                                                                                                                                | **C**                                                                           | JS draws from events. Source of volume and brightness events is open (Q7)                                                                                                        |
| 18  | A11y UI: magnifier, sticky/slow/bounce/mouse keys                                                                                    | Rust: `gnoblin-a11y` policy, magnifier drawn by `gnoblin-render`; no popups                                            | **Yes**: the user cannot fix a broken shell without it                    | Not applicable                                                                                                                                                                                    | **A**                                                                           | Must work with no script. Configured by settings only. The Access consent is row 1; the pad-help overlay is shell-owned (row 19)                                                 |
| 19  | Gesture hints, pad help                                                                                                              | `gestures.lua` follows fingers (workspace slide); no hint overlay; pad help is an event for shell clients              | No                                                                        | No                                                                                                                                                                                                | **C**                                                                           | Visual feedback for a script-owned gesture                                                                                                                                       |
| 20  | Recovery panel ("shell did not appear")                                                                                              | docs only (`bring-your-own-shell.md`); no code found, _unverified_                                                     | Yes by definition                                                         | No secret                                                                                                                                                                                         | **A** if it exists                                                              | Its whole purpose is the no-shell case                                                                                                                                           |
| 21  | Capture / mic / camera privacy indicator                                                                                             | state served by `gnoblin-service-privacy` (`org.gnoblin.Privacy`) and consent audit; no stock Lua drawer found         | Should be                                                                 | Yes: a script must not be able to hide that a screen is shared                                                                                                                                    | **B**, tiny Rust dot as fallback and not hideable while a stream is active (Q8) | Not in the assignment list but it is the same trust class as row 1                                                                                                               |

Summary: A = rows 1, 3, 5, 6, 7, 18, 20. B = 2, 4, 9, 21. C = 10 to 17, 19. Row 8 is not ours.

New Rust work implied: polkit dialog and password path (6), lock fallback (9), wire the chooser UIs or keep routing to gtk (3, 4), `DialogStyle` plumbing (section 6), picker offer fields (2). The built-in `prompt.secret` dialog (5) is done, see below.

**Row 5 status: the Rust built-in exists.** `gnoblin-portal-consent::SecretUi` is the egui card (lock, the request's title, question and subject, a masked entry, Cancel and Unlock), and `gnoblin-compositor/src/ipc/secret.rs` `SecretService` shows it as the compositor's `Modal` (secure overlay above everything, every key and click taken by the compositor, typed text sent only to a modal that asks for it). It serves a `prompt.secret` request when no shell claims the slot, and, per section 5, the compositor keeps the request and its deadline: a claimant that leaves before answering hands the same request (same id, same deadline) to the built-in, and a claimant that answers first wins. A claimant's decline is still its answer and ends the request: the stock shell's Escape is a plain decline, so reading it as "cannot draw" would open a second dialog over a prompt the user dismissed. The Lua `prompts.lua` is unchanged and still claims the slot on a stock session. Not done: the claimant's half-budget for a connected but wedged claimant (that request waits for its own deadline), the `SecureEntry` tier (4.3), polkit (row 6, which needs the owner decision on who verifies the password).

## 4. Proposed JS API shape

One namespace, `gnoblin.dialogs`, built on the existing slot and request machinery (`slots.claim`, `request.offered|claim|answer|decline`, `gnoblin-lua/src/slot_link.rs`, to be re-hosted in `gnoblin-js`). It follows spec §4: typed listener classes, first argument is the originating object.

```ts
type DialogKind = "consent" | "screencast" | "filechooser" | "appchooser" | "secret" | "auth" | "location";

declare namespace gnoblin.dialogs {
  style: DialogStyle;                                  // config-document backed (section 6)
  readonly pending: Collection<DialogRequest>;         // offered, not yet settled
  addListener(l: DialogListener): void;                // observe, restyle, decide by policy
  claim<K extends DialogKind>(kind: K, opts: ClaimOptions, draw: (r: RequestOf<K>) => Promise<void>): Claim; // replace
}

abstract class DialogListener {                        // all methods optional, like ClientListener
  onConsentRequested?(r: ConsentRequest): void;
  onScreenCastRequested?(r: ScreenCastRequest): void;
  onSecretRequested?(r: SecretRequest): void;          // may restyle, may not read the secret
  onAuthRequested?(r: AuthRequest): void;
  onLocationRequested?(r: LocationRequest): void;
  onShown?(r: DialogRequest): void;
  onSettled?(r: DialogRequest, outcome: DialogOutcome): void;  // never carries the answer value
}

interface DialogRequest {                              // fields come from request.offered
  readonly id: string;  readonly kind: DialogKind;  readonly deadline: number;
  readonly app: { id: string | null };                 // compositor-verified; no setter
  readonly title: UntrustedText;  readonly message: UntrustedText;  // app-written: branded, must be sanitised
  style: Partial<DialogStyle>;                         // per-request override, clamped by Rust
  deny(reason?: string): void;                         // policy answer, consent kinds only
  decline(reason?: string): void;                      // hand to Rust built-in
}
```

**Typed requests.** `ScreenCastRequest` adds `sources: readonly Source[]` (`monitor | window | view`, with node ids) and `types`; a listener may _remove_ sources (`r.sources = r.sources.filter(...)`) and `r.preselect(s)`; it cannot add one (Rust checks the answer is a subset of what it offered, as `validate.rs` does for choosers). `SecretRequest` carries `kind: "passphrase" | "confirm"` and `expiresIn` (120 s today).

**Three levels of control, three levels of authority.**

| Level               | What a script does                                                  | Authority needed                                               |
| ------------------- | ------------------------------------------------------------------- | -------------------------------------------------------------- |
| Restyle             | set `gnoblin.dialogs.style` or `r.style`; Rust draws                | none beyond running in the runtime                             |
| Intercept by policy | `onConsentRequested(r) { r.deny() }`, narrow `sources`, `preselect` | the listener. `r.allow()` is **not** offered: auto-allow is Q2 |
| Replace             | `gnoblin.dialogs.claim(kind, ...)`: the script draws and answers    | the owner connection's `slots.claim` (4.2) plus a capability   |

### 4.1 Rules that hold for every replacement (unchanged from today)

Deny is the default; only the exact answer `allow` grants; an early allow is a refusal (500 ms); identity is the compositor's `app.id`; any failure goes to the Rust dialog; a locked session refuses with `locked`; grants are the compositor's record. These are in Rust (fact 2) and no JS API can waive them, so the JS typings simply have no way to express anything else: `settle(answer: "allow" | "deny")`.

### 4.2 How replacement is authorised

- **Process level:** the runtime holds the second owner socket (`connect_session_shell`, SO_PEERCRED uid check in `ipc/privileged.rs`). Only that connection can claim a slot, first claim wins, a second holder is refused (`slot.rs`, `portal_provider.rs` test). No change.
- **Module level:** every user module shares that one connection, so the compositor cannot tell `init.ts` from an imported third-party file. The runtime must gate `claim()` itself. Spec §3/§18 mention "permissions" and "unauthorised capabilities" but do not define them (_unverified_), so propose one capability string per kind, granted in `init.ts` (`gnoblin.permissions.grant("./my-prompts.ts", ["dialogs.replace:screencast"])`), with the default for the stock library only. This is hygiene against a careless extension, not a boundary against a hostile one: any module in the runtime can also read all keys through bindings (Q5).

### 4.3 The secret-entry caveat

A JS-drawn passphrase prompt sees every keystroke as a JS string, which cannot be wiped, and any `console.log` or inspector dump (spec §18 inspector) can leak it. Two tiers:

1. **Rust built-in (row 5, 6):** the default. No script sees the secret.
2. **`SecureEntry` node (new native class):** the script draws the card, scrim, labels and buttons; keys typed while a `SecureEntry` holds focus go from the compositor straight into a zeroising `SecretText` buffer (`gnoblin-secret`). The script receives only `length` and a `submit` signal with an opaque `SecretHandle`, and passes it to `r.answer(handle)` or `lock.authenticate(handle)`. A claim for `secret` or `auth` is refused unless the dialog tree contains a `SecureEntry`. This is the same shape as `Lock.authenticate`'s `secret` parameter today.

Honest limit: a hostile script with owner rights can still bind every key chord or draw a fake prompt with a plain `Entry` and keep the text. `SecureEntry` protects against accidents (logging, crash dumps, string copies) and against a replacement that _claims_ a real request, not against a malicious `init.ts`. Replacing `auth.polkit` password collection is not offered in v1.

## 5. Crash, hang and hot reload without losing a request

Principle: **the compositor owns the request and its deadline; JS only holds a lease on it.** A request is never stored only in a JS heap.

| Event                                            | What happens                                                                                                                                                                                                                                                                                                                                                                                                                                                                                             | Status                                                                               |
| ------------------------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------ |
| Runtime absent at boot                           | no claim: Rust built-in serves (kinds 1, 3, 5, 6, 7 once built); kinds with no built-in refuse at once                                                                                                                                                                                                                                                                                                                                                                                                   | works for `portal.ui`; **new** for 5, 6                                              |
| Runtime crashes while it holds an offer          | connection closes -> slot released, shell nodes destroyed next frame; the request returns to Offered and the Rust dialog opens with the remaining deadline. The Rust `Modal` is not a tree node, so it is unaffected                                                                                                                                                                                                                                                                                     | works for `portal.ui` (`provider_settled` "claimant that left"); **new** for 5, 6, 9 |
| Runtime alive but wedged                         | the claimant's half-budget (`routable`) expires, then Rust. The interrupt handler (spec §1.13, §18 budgets) should end a runaway handler first                                                                                                                                                                                                                                                                                                                                                           | exists for `portal.ui`                                                               |
| Hot reload (spec §18)                            | the process and socket persist. At the commit boundary the old generation calls `request.release(id, "reload")` (**new verb**) for each request it holds: the request returns to Offered and is held for at most 2 s. If the new generation re-claims the slot in that window the request is re-offered with `redelivered = true` and the same id and deadline; otherwise Rust draws it. The old generation's dialog nodes are destroyed in the same transaction. Typed text is lost; say so in the docs | **new**                                                                              |
| Reload fails (spec §18 step 5)                   | previous generation stays: no release, no re-offer                                                                                                                                                                                                                                                                                                                                                                                                                                                       | by construction                                                                      |
| A claimant returns while the Rust dialog is open | the Rust dialog is not taken away. It finishes; later requests go to the claimant                                                                                                                                                                                                                                                                                                                                                                                                                        | **new** rule, avoids flicker and a handoff race                                      |
| Late answer from a gone claimant                 | refused `request-finished`                                                                                                                                                                                                                                                                                                                                                                                                                                                                               | exists (`requests.md`)                                                               |
| Lock requested while runtime is down             | cover goes up in Rust; the Rust lock fallback (row 9) shows the entry; when the runtime returns, `Lock.on_ready` (`lock.lua:Lock:on_ready`) sees `state != not-locked` and the script screen replaces the fallback                                                                                                                                                                                                                                                                                       | fallback **new**                                                                     |

Interim shortcut that needs no new verb: on reload, `decline("handoff")` to the Rust dialog (works for `portal.ui` today, per `replace-built-in-dialogs.md`). It visibly swaps the dialog mid-question, so it is the stopgap.

## 6. Theming: `init.ts` -> Rust-drawn popups

The Rust dialogs must be styled **before** and **without** the runtime, so the style cannot live only in a JS object. Use the config document, which is already validated, merged, persisted as last-good (`gnoblin-config/src/last_good.rs`) and committed by the runtime at boot (current-runtime-map.md section 1). Precedent: `PortalService::configure(&Config)` already reads `appearance.color-scheme` from it.

```
init.ts: gnoblin.dialogs.style = {...}   (typed; serialised by gnoblin.configure)
   -> config document key `dialogs.style`  (new section in crates/gnoblin-schema/schema/config.toml, validated by gnoblin-config)
   -> compositor `PortalService::configure` -> `Setup { scheme, style }` -> DialogShell (egui Visuals/Style)
   -> JS stock library (B surfaces) reads the same object: one source of truth
```

```ts
interface DialogStyle {
    scheme: "auto" | "light" | "dark"; // auto = appearance.color-scheme (today's behaviour)
    background: Color;
    foreground: Color;
    accent: Color;
    danger: Color;
    border: Color;
    scrim: Color; // dim layer behind the dialog
    radius: number;
    shadow: Shadow | null;
    font: { family: string; size: number };
    buttons: { order: "deny-first" | "allow-first"; layout: "end" | "stretch" | "stacked" };
    width: number;
}
```

Rust **clamps** a style so a restyle cannot become a disguise (validated at config commit, and again for `r.style`): text/background contrast at least 4.5:1 and button/background at least 3:1; font size at least 12; Deny never smaller than Allow; Deny keeps initial focus; the arm delay (500 ms) and the identity line are not styleable; `scrim` alpha at least 0.4 for consent. A style that fails is refused as a whole with the key path, like any config error (`gnoblin-config` path-named errors), and the previous style stays.

Implementation notes: egui takes colours and rounding through `Visuals`/`Style` (the one hook is `DialogScheme::theme()` in `gnoblin-portal-ui/src/ui.rs:40`; it must take a `DialogStyle`). Button order and layout need code in each screen crate (`gnoblin-portal-consent`, `-appchooser`, `-filechooser`). Font family by name needs font lookup the egui crate does not do today; `gnoblin-paint` already finds system fonts first with an embedded Droid Sans fallback (`gnoblin-paint/fonts/README.md`), _unverified_ whether `portal-ui` can reuse it. Long term (Q9), draw the built-ins with `gnoblin-paint` too, so there is one painter and one style engine, and egui goes away (O2 kept egui "for the built-in portal dialogs").

## 7. `init.ts` sketches

### 7.1 Restyle every dialog

```ts
// ~/.config/gnoblin/init.ts
gnoblin.dialogs.style = {
    scheme: "dark",
    background: "#1e1e2e",
    foreground: "#cdd6f4",
    accent: "#89b4fa",
    danger: "#f38ba8",
    border: "#45475a",
    scrim: "#000000aa",
    radius: 14,
    shadow: { x: 0, y: 8, blur: 24, color: "#00000066" },
    font: { family: "Inter", size: 14 },
    buttons: { order: "deny-first", layout: "end" },
    width: 480,
};
// One kind differs. Merged over the global style; clamped by Rust like the rest.
gnoblin.dialogs.style.for("secret", { accent: "#f9e2af" }); // `for` is a typed helper
```

### 7.2 Intercept the screencast picker

Policy only (Rust still draws; section 4 "intercept"):

```ts
import { DialogListener, ScreenCastRequest } from "gnoblin";

class PickerPolicy extends DialogListener {
    onScreenCastRequested(r: ScreenCastRequest) {
        if (r.app.id === "com.obsproject.Studio") {
            r.preselect(r.sources.find((s) => s.kind === "monitor" && s.primary));
        }
        // Never offer a window that the user marked private. Narrowing is allowed, adding is not.
        r.sources = r.sources.filter((s) => !(s.kind === "window" && s.client?.tags.has("private")));
    }
}
gnoblin.dialogs.addListener(new PickerPolicy());
```

Replace it (the script draws live thumbnails; needs `dialogs.replace:screencast`; the compositor still checks the answer):

```ts
gnoblin.dialogs.claim("screencast", { name: "stock-picker" }, async (r) => {
    const dialog = new Dialog({ style: gnoblin.dialogs.style });
    for (const s of r.sources) dialog.add(new SourceTile({ source: s, thumb: new View({ source: s.node }) }));
    const choice = await dialog.result; // { allow: true, sources } | { allow: false }
    choice.allow ? r.settle("allow", choice.sources) : r.settle("deny");
    // throw, return without settling, or miss the deadline -> Rust consent serves it (section 5)
});
```

### 7.3 Replace the passphrase prompt

```ts
// Needs: permissions.grant("./prompt.ts", ["dialogs.replace:secret", "secure-entry"])
gnoblin.dialogs.claim("secret", { secureEntry: true }, async (r) => {
    const entry = new SecureEntry({ masked: true }); // keys never reach this script (4.3)
    const dialog = new Dialog({
        title: sanitise(r.title),
        subtitle: r.app.id ?? "unknown application", // never the app's own text as identity
        body: [entry],
        buttons: ["Cancel", "Unlock"],
    });
    const out = await dialog.result;
    if (out.button === "Unlock")
        r.answer(entry.takeSecret()); // SecretHandle, consumed once
    else r.decline("cancelled"); // keyring stays locked
});
```

Caveat in the sketch: with a plain `Entry` instead of `SecureEntry` the claim is refused (`secret-needs-secure-entry`). With no claim the Rust prompt serves.

### 7.4 Disable the window menu

Menu is C: a stock-library setting, not a Rust switch.

```ts
gnoblin.stock.windowMenu.enabled = false; // replaces `seat.data...` flags of the Lua stock shell
// or keep it but remove entries / veto a request:
class NoMenu extends ClientListener {
    onMenuRequested(client: Client, req: MenuRequest) {
        req.handled = true;
    } // titlebar right click, menu key, client request
}
gnoblin.clients.addListener(new NoMenu());
```

Today's equivalents: `seat.data.prompts = false` and `seat.data.alerts = false` in `shell.lua`, and replacing `Window:menu_items` / `Window:show_menu` (`docs/guides/stock-window-menu.md`).

### 7.5 Status (P6 part 1)

| Item                                                                | Status                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                            |
| ------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `dialogs.style` config key (section 6)                              | Done: `radius`, `accent`, `font.family`, `font.size`, `button-layout`, `scale` in `crates/gnoblin-config/schema/config.toml`; user doc `docs/config/configure/dialogs.md`. Named system fonts are not loaded: `monospace` selects the monospace face, any other name the embedded one.                                                                                                                                                                                                                            |
| Clamping (`gnoblin_portal_ui::style`)                               | Done, drawn by `ConsentUi` and `SecretUi`: contrast 4.5 (text) and 3 (accent, corrected toward white or black), button floor 88x32 and Deny never smaller than Allow, font 12-32, scale 1-3 lowered until the dialog fits, radius at most 24. Arm delay, requester line, lock icon: no setting.                                                                                                                                                                                                                   |
| `dialogs.observe` slot (compositor)                                 | Done: a shell claims it and is shown each consent and passphrase request the built-in dialog is about to draw, for 250 ms (`OBSERVE_MS`). Its answer `deny` refuses by policy; anything else, silence, a decline or a departure leaves the request, same id and deadline, to the built-in dialog. It never draws, never answers `allow`, never receives or gives a passphrase. Offer frame: slot `dialogs.observe`, title `consent:<interface>` or `secret`, message the app's text, subject the verified app id. |
| `gnoblin.dialogs` in `data/js/gnoblin/dialogs.ts`, `DialogListener` | Not done: the JS shell holds no slot connection yet (`JsShell` has no `SlotLink`), so there is nothing to attach a listener to. Needs a second owner connection in `JsRole`, a `SlotLink` in `JsShell` polled in `frame`, and `retire` answering `pass` for each held observation and releasing the slot.                                                                                                                                                                                                         |
| `defineConfig({ dialogs: { style } })` reaching the compositor      | Not done: the JS runtime commits no configuration document today (the Lua host commits it). The JS role needs `config.current` then `config.commit` with the merged section, or to own the document.                                                                                                                                                                                                                                                                                                              |

## 8. Open questions for the owner

1. **Port `prompts.lua` or not?** Recommended: no. Rust draws consent (row 1), themed by `DialogStyle`; the stock TS library claims nothing. This reverses today's default (the Lua dialog is what users see) but fits O3, which names the built-in the default and does not require the stock shell to claim. Say yes if the dialogs must be pixel-identical to the rest of the shell.
2. **May a script answer `allow` without a person?** A user policy ("always allow OBS to cast") needs an answer type that skips the 500 ms rule. Proposal: `allow-by-policy`, only for kinds listed in a config key, always in the audit log and privacy indicator. Or: never, and use stored permissions in the Rust authority. Needs an owner decision.
3. **Do we build the Rust built-ins for `prompt.secret`, polkit and location now?** Without them a runtime crash leaves the keyring locked and polkit with no UI.
4. **Title bars without a runtime:** accept "windows have no titlebar until the guardian restarts the runtime", or add a minimal Rust titlebar from the same paint list (`gnoblin-paint` already rasterises it)?
5. **Is the user's `init.ts` trusted like the user?** If yes, `SecureEntry` and per-module permissions are hygiene only. If third-party extensions are expected, the runtime needs a real permission model (spec §17 does not define it).
6. **Polkit:** `gnoblin-service-polkit` replies to `BeginAuthentication` from a provider's answer and says the password never reaches it. Real agents run polkit's setuid helper and report with `AuthenticationAgentResponse2`; I did not find that path (_unverified_, did not read `subject.rs` or the compat tests). Who verifies the password in Gnoblin's design?
7. **Event sources for OSD and notifications:** volume and brightness are not compositor state. Should the JS runtime read PipeWire/logind through services, or do external clients (Bingux) own OSD?
8. **Privacy indicator:** compositor-drawn and not hideable (my recommendation), or a normal JS surface fed by `org.gnoblin.Privacy`?
9. **Retire egui?** Drawing the built-ins with `gnoblin-paint` would give one painter and a real font/colour/radius pipeline, at the cost of re-doing `gnoblin-portal-ui` (egui widgets, text edit) and re-opening O2.
10. **Hold time on reload:** 2 s before the fallback appears. Acceptable, or show the Rust dialog immediately and swap after the claim returns?
11. **Chooser UIs:** keep routing file and app choosers to xdg-desktop-portal-gtk/kde (`gnoblin-portals.conf`), or wire the built-ins? The JS design above does not depend on it.
12. **Lock fallback scope:** password entry only, or entry plus clock and the notification count? Anything more makes the Rust lock a second lock screen to maintain.
