# End-to-end passphrase prompts with gnome-keyring

Status: **not proven end to end.** The Agent37 run has not happened yet: the instance
was queued behind the account's five-instance limit. Everything below marked _code_ is
read from the source at `origin/rust-compositor` `0381411ca`. Nothing marked _code_ has
been run on a live session.

## The requirement

Kieran's requirement (b): keep another keyring (gnome-keyring, KeePassXC, KWallet) as
the Secret Service backend, and let Gnoblin only draw the keyring and pinentry dialogs.
The config that selects this is `prompts.enabled = true` with `keyring.provider` set to
the other keyring. The Secret Service side belongs to `design/keyring.md`. This document
covers only the prompt path.

Config form used. Main's brief names `keyring = { enabled = false }` with `prompts = { enabled = true }`, which replaces `keyring.provider`. That form is not on `origin/rust-compositor` at `0381411ca`: the schema still defines `keyring.provider` (text, nonempty) and no `keyring.enabled`. The run therefore uses the old form: `keyring.provider = "gnome-keyring"` with `prompts.enabled = true`. Switch to the bool form once the KeyringOff commit is on origin, and re-run.

## The chain

1. gnome-keyring or `pinentry-gnome3` asks `org.gnome.keyring.SystemPrompter` on the
   session bus. _code:_ `crates/compat/gnoblin-compat-dbus-gnome-keyring-prompter`.
2. The services role links the prompter only when `prompts.enabled` is true.
   _code:_ `crates/gnoblin-services/src/prompts.rs`,
   `crates/compat/gnoblin-compat-host/src/services/mod.rs`.
3. The adapter turns each gcr prompt into a `prompt.secret` offer through `Host::offer`.
   _code:_ `agent.rs` and `state.rs` in the prompter crate.
4. The compositor routes the offer to the claimant of `prompt.secret`. _code:_
   `crates/gnoblin-compositor/src/ipc/requests.rs`, `provider_of`.
5. A claimant answers over IPC `request.answer`. The adapter decrypts the answer and
   returns it to gcr.

## Finding 1: step 4 has no claimant in shipped code

`provider_of` returns `Provider::None` when no connection has claimed the slot. Only a
connection that claims the slot can receive the offer. A search of the crates finds
`slot.claim` used by:

- the client library,
- the tests,
- the services role, for `auth.polkit`, `location.agent` and `idle.policy`.

No product shell calls it for `prompt.secret`.

With no claimant, the `Provider::None` arm refuses the offer at once. The adapter maps
that to `Unavailable::NoProvider` and refuses the gcr prompt without waiting. So with
`prompts.enabled = true` the prompt is refused immediately. It is not drawn, and it
does not wait for the 120-second deadline.

This is fail-closed, which is the right direction, but the feature does not do what
requirement (b) asks.

## Finding 2: the documented Lua API does not exist

The Lua runtime drops prompt requests. The dispatch in `crates/gnoblin-lua/src/host.rs`
matches the three request events and does nothing with them. The match arm carries the
comment "The runtime offers no requests in this slice". `crates/gnoblin-lua/src/lib.rs`
says the same.

So the Lua surface that `docs/shell-api/passphrase-prompts.md` describes is absent:

- `gnoblin.prompt.requested` is never delivered to Lua.
- `request:answer` and `request:cancel` have no binding.
- No `gnoblinctl` verb exists for prompts.
- A grep for `prompt.respond` and `prompt.cancel` finds nothing in the crates.

The host comment in `dispatch` said an unanswered request "fails closed at its deadline
in the broker". That is true only once a claimant has received the offer. With no
claimant, the broker refuses at once (Finding 1).

## Finding 3: what the tests prove

The adapter path is tested with a stand-in for the shell, not a real one:

- `crates/compat/gnoblin-compat-dbus-gnome-keyring-prompter/tests/dbus.rs`,
  `a_prompt_is_offered_answered_and_encrypted`: gcr-side D-Bus, with a fake host.
- `tests/ipc.rs`, `a_shells_answer_reaches_the_prompter_encrypted`: a client connection
  claims `prompt.secret` and answers over IPC.
- `crates/gnoblin-compositor/src/ipc/requests.rs`,
  `a_host_offer_to_a_shell_claimed_slot_goes_to_the_shell`: routing only.

None of these runs the gnome-keyring daemon, `pinentry-gnome3`, or a real shell.

## Plan for the Agent37 run

The run answers one question: with `keyring.provider = "gnome-keyring"` and
`prompts.enabled = true`, does a gcr prompt reach a drawn dialog and come back with an
answer?

1. `a37-gate.sh up PromptsE2E`, then `run` the existing prompter tests and the
   compositor requests tests, so the baseline is recorded.
2. Start a private session bus (`dbus-run-session`). Start `gnome-keyring-daemon
--start --components=secrets`. Confirm `org.freedesktop.secrets` is owned by
   gnome-keyring, not Gnoblin. Use `busctl list` and `busctl status`.
3. Start the services role with `prompts.enabled = true`. Confirm
   `org.gnome.keyring.SystemPrompter` is owned.
4. Ask gnome-keyring for a locked collection. Capture the offer with `dbus-monitor` and
   the compositor log. Expected, from the code: `NO_PROVIDER`, returned at once.
5. Repeat with a shell-stand-in client that claims `prompt.secret` and answers. This
   proves the adapter answer path on a real bus, not that a shell draws the dialog.
6. Repeat with `prompts.enabled = false`. Expected: the prompter is not linked, and
   nothing owns `org.gnome.keyring.SystemPrompter`.

Every step must have its log saved under `~/dev/lanes/PromptsE2E-logs/`. A step with no
log is not reported as passing.

## Big items, not fixed in this lane

| #   | Item                                                               | Why it is big                                                                                | Owner                                                                     |
| --- | ------------------------------------------------------------------ | -------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------- |
| 1   | Lua `gnoblin.prompt.requested`, `request:answer`, `request:cancel` | New event delivery and a request object in `gnoblin-lua`; the host drops requests on purpose | Lua / shell-API lane (no owner named in the roster; needs Main to assign) |
| 2   | A shell claims `prompt.secret` and draws the dialog                | Depends on item 1; the shell has no prompt surface yet                                       | Shell lane (unassigned)                                                   |
| 3   | `gnoblinctl` prompt verb                                           | Depends on item 1                                                                            | `gnoblinctl` owner (unassigned)                                           |
| 4   | Default provider for `prompt.secret`                               | Design decision: a default would let any shell answer, so it stays `None` until decided      | Main                                                                      |

## Small items, fixed in this lane

- The passphrase-prompts page now carries a status note: the Lua API is not implemented,
  and with no claimant the prompt is refused at once.
- The `dispatch` comment in `host.rs` now says when the broker refuses: at once with no
  claimant, and at the deadline once a claimant has the offer. Comment only; no behaviour
  change.

Both are documentation fixes. Neither has a failing test, because no behaviour changed.
The Agent37 run is still required before any claim of end-to-end behaviour.
