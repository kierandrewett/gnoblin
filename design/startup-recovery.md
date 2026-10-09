# Startup recovery

Gnoblin bootstraps native compositor services with the complete embedded Lua
configuration. The guardian never evaluates user Lua. After HELLO and the
initial default autostart snapshot, the worker stages the user configuration
through the existing live reload transaction.

The startup transaction has no external API caller. Its result commits the
candidate Lua runtime, saves accepted settings, applies portal routes and
updates config-owned autostart clients. A rejected candidate publishes the error
to the integrated ImGui recovery panel, then stages saved accepted settings for
the same config path through another native transaction. The saved runtime uses
embedded input callbacks because settings cannot restore Lua closures. Defaults
remain active if the saved snapshot is missing, invalid or rejected. Failed user files are never rewritten.

A worker that dies before producing its initial CONFIG gets a bounded retry
using embedded defaults and a fresh private autostart channel. Replacement
workers retain an autostart channel for later reloads. Saved-runtime recovery
tries embedded defaults if constructing the saved fallback fails.

This addresses configuration rejection before native startup and missing
recovery notices. It does not make loader, GPU, seat or filesystem failures
recoverable by Lua. A native crash during transaction application is still a
compositor fault and must be diagnosed independently. Post-handshake worker
resume includes the same display environment as the initial handshake and
requires a snapshot matching the compositor. Worker recovery must carry the
accepted document, subscriptions and counters from native suspension through a
private anonymous descriptor. The replacement must not evaluate the failed user
files. It reconnects using that exact snapshot, gates callbacks during the
transition, and transactionally installs embedded defaults before dispatching
input. The config diagnostic must survive that transaction. Native snapshot
validation remains strict; resuming directly with a guessed default document
can fail when native accepted a candidate just before the worker died.

Source and installed-artifact evidence are separate from fresh-login evidence.
A successful build does not prove on-screen recovery or login stability.

The October 6 login failures occurred before native bootstrap: settings
serialization confused device keyboard settings with callback declarations,
then pointer registration used a Lua stack index invalidated by removing a
lower stack item. Both errors also affected the embedded configuration. After
correcting those paths, the installed worker produced its initial CONFIG on
private channels in 17 ms. That observation does not prove real-seat login or
on-screen diagnostics.

## Crash evidence

The guardian rotates private, bounded diagnostics at the beginning of each
session attempt under `$XDG_STATE_HOME/gnoblin/`:

- `session-last.log` and `session-previous.log` record guardian milestones,
  child PIDs, and decoded exit statuses.
- `compositor-last.log` and `compositor-previous.log` contain compositor
  stderr. The guardian also forwards that stderr to its original stream for
  normal journal capture.

Each file is mode `0600` and capped at 64 KiB. The records deliberately omit
environment values, Lua source, and configuration documents. They make a
return to the greeter actionable when the compositor dies before the ImGui
overlay can be drawn.

Config-owned autostart spawn failures and unsuccessful exits publish a recovery
notice with the component name. Existing config diagnostics are preserved.
XDG autostart remains outside the config-owned process registry.

## Diagnostic presentation

The recovery overlay follows the stage size explicitly. Mutter window groups
have zero preferred size, so actor expansion alone does not allocate it. The
panel owns pointer presses only inside its painted bounds and retains their
releases outside the panel. Hover motion and input outside it remain available
to clients. The console remains modal. Diagnostic surfaces hide and clear
queued input while session lock is active.

Open config launches the associated folder application asynchronously. Terminal
launch uses xdg-terminal-exec when present, otherwise the first available
supported terminal. Reload uses the existing native runtime transaction. ImGui
mouse transitions are queued as events arrive, and a bounded frame tick while a
button is held drains quick press/release events across paints.

The diagnostic implementation and ImGui are compiled into the Gnoblin executable,
including the statically registered compositor plugin type. No dynamic Gnoblin
plugin module is needed by the session.

## Startup channel ordering

The ready pipe and the autostart socket are separate channels. A supervisor
must retain each observation until both have arrived; polling one channel
before a send and the other afterward is valid. An absent autostart snapshot
at READY is not proof of a failed worker. The supervisor waits with a deadline
and replaces the worker on expiry while keeping the compositor alive.

Recovery sends its default autostart snapshot before signaling READY. A resumed
supervisor still provisions a fresh autostart update socket so defaults can stop
config-owned clients from the previous accepted configuration.
