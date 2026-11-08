# Session and runtime health

> **Status:** not available yet: `gnoblin.capabilities`, `gnoblin.config.reloaded`, `gnoblin.focus.history`, `gnoblin.focus.policy`, `gnoblin.input.devices`, `gnoblin.layers`, `gnoblin.runtime.reload_config`, `gnoblin.runtime.status`, `gnoblin.session.activity`, `gnoblin.session.hibernate`, `gnoblin.session.lock`, `gnoblin.session.logout`, `gnoblin.session.poweroff`, `gnoblin.session.reboot`, `gnoblin.session.suspend`. The sections that use them describe the target, not current behaviour.

`gnoblin.session:status()` returns a record with these fields:

- `state` is `"running"` while the compositor answers the request.
- `lock_available` says whether Mutter can report its lock state.
- `revision` identifies the current lock-state snapshot. It starts at `0` and
  advances when the lock state changes. The matching event has the same
  revision.
- When `lock_available` is true, `lock_state` is `unlocked`, `covering`,
  `locked`, or `failsafe`.

When lock state is unavailable, the record omits `lock_state`; unavailable does
not mean unlocked.

The read cannot report why a stopped session exited. The socket is unavailable
after the compositor stops. The record remains available while the Lua
supervisor is recovering. It reports compositor state, not supervisor health. Subscribe to
`gnoblin.session.lock_state_changed` for lock transitions.

`gnoblin.session.logout()` takes no arguments and returns an operation whose
successful value is `{accepted = true}`. After the compositor confirms that
operation, the supervisor exits successfully and stops the compositor. The
session wrapper then stops Gnoblin's user services and returns to the login
manager. A socket caller may receive EOF while the session is closing.

`gnoblinctl lua` exposes this operation and `gnoblin.session.lock()` too. The
lock result reports that a subscribed shell client received the request; check
`gnoblin.session:status()` to confirm the compositor entered a locked state.

These methods take no arguments and return operations:

- `gnoblin.session.suspend()` asks logind to suspend.
- `gnoblin.session.hibernate()` asks logind to hibernate.
- `gnoblin.session.reboot()` asks logind to reboot.
- `gnoblin.session.poweroff()` asks logind to power off.

The compositor reads the matching logind capability, then asks logind for the
action. The operation finishes with logind's answer, so it does not report
success before logind replies. It succeeds with `{accepted = true}` when
logind accepts the request. The Lua runtime keeps running after a suspend or
hibernate. For a reboot or power off, logind ends the session after it
accepts.

Any sleep, including `systemctl suspend` and hibernate, locks the session first. On logind's `PrepareForSleep(true)` Gnoblin locks while it still holds its sleep inhibitor, then releases the inhibitor so the sleep goes ahead. A refused lock still releases the inhibitor, so a refusal never blocks the sleep.

A refused or failed request ends the operation with an error:

- `denied`: the capability answer is `no`, or polkit denies the action (`AccessDenied`).
- `unsupported`: the capability answer is `na`, or logind has no such action.
- `busy`: logind is already running another power action.
- `unavailable`: logind is not on the system bus.
- `timed_out`: logind did not answer in time (5 seconds for the capability, 30 for the action).
- `internal`: any other D-Bus error from logind.

The message names the D-Bus error, for example
`logind could not suspend: org.freedesktop.DBus.Error.AccessDenied: ...`.
Logind may answer `challenge` to the capability query. Gnoblin then asks for
interactive authorization with the action.

The focus reads are available as `gnoblin.focus.history(filter)` and
`gnoblin.focus.policy()`. See [Focus and activation](/shell-api/focus-activation)
for their fields and filter values.

For configuration path and default reads or restoring the default tree, see
[Configuration files](/config/configure/config-files). `gnoblinctl init` remains
a command-line setup action.

The [compositor bridge](/bridge/) documents the socket request format.

## Runtime health and reload

`gnoblin.runtime.reload_config()` takes no arguments. It validates and applies
the complete Lua configuration while the session is running. Native settings,
policies, autostart entries, and Lua callbacks are updated as one reload
transaction. If loading or applying the candidate fails, Gnoblin keeps the
previous configuration active.

Call `gnoblin.runtime.status()` to read worker health:

```lua
local status = gnoblin.runtime.status()
print(status.state, status.generation)
```

It returns a read-only record with `state` and `generation`. The state values are:

- `starting` before the worker connects;
- `running` while it serves requests;
- `restarting` while Mutter suspends it for replacement;
- `unavailable` when the supervisor is disconnected, stopping, or reports that
  it will not restart the worker.

The compositor answers the read while the Lua worker is restarting, so shell
clients can use `gnoblinctl lua` to poll recovery. A call from the live Lua
runtime reports `running` and that worker's active generation.

`generation` identifies the runtime configuration accepted by Mutter. It
stays unchanged across worker recovery and changes when a different
configuration is accepted.

Subscribe to `gnoblin.runtime.status-changed` to watch those values. The event
also includes `sequence` and monotonic `time`. It is emitted when the worker
starts serving, Mutter suspends it for replacement, the replacement finishes
republishing state, or a new configuration generation is accepted. Failed or
rolled-back reloads do not change the reported generation.

```lua
gnoblin.on("gnoblin.runtime.status-changed", function(event)
    print(event.state, event.generation)
end)
```

Events are not replayed, so subscribe before reading `runtime.status()` and
read it again after reconnecting.

A Lua worker cannot run callbacks while suspended. External clients can read
`restarting` during that time.

When the supervisor reports that it will not restart the worker, Mutter reports
`unavailable` and emits a final event while it remains alive. The event is not
sent if the compositor exits first and closes the socket.

The native open-animation matcher uses updated rules for windows mapped after
reload. Reload does not replay open animations for windows already mapped.

The Lua `Operation` completes after the candidate config has loaded, validated,
and been staged. Its `runtime_generation` is the generation assigned if the
candidate commits; it does not confirm commit. Mutter waits for active-runtime
operations and deferred callbacks before swapping runtimes.

`gnoblinctl lua` also exposes `gnoblin.runtime.reload_config()`. It waits for
the compositor operation and returns its result as a deeply read-only value.

`gnoblin.config.reloaded` signals that the candidate became active. If the
session stops first, the candidate is discarded and that event is not sent, even
if the Lua operation already reported successful staging.

After the commit, the runtime also emits `gnoblin.focus.policy-changed` when the
effective focus policy changed. This event carries the committed policy snapshot
and configuration revision. A failed reload emits `gnoblin.config.reload_failed`
while the previous runtime remains active.

### Reload warnings

Check `warnings` in the result of `gnoblinctl config reload` or
`gnoblin.runtime.reload_config()` from `gnoblinctl lua`. It is a list of
diagnostic strings for unknown top-level sections and protocol names in the
accepted config. The reload succeeds with `ok = true`; unsupported names
have no effect. A clean config returns an empty list.

For example, open `gnoblinctl lua` inside your session and enter:

```lua
for _, warning in ipairs(gnoblin.runtime.reload_config().warnings) do print(warning) end
```

The `gnoblin.config.reloaded` Lua event carries the same `warnings` list.
Use that event for feedback after a reload requested by a running Lua handler;
its `Operation` result only confirms staging.

Warnings use normalized names: a misspelled `not_a_section` appears as
`not-a-section`. The list contains up to 64 diagnostics, followed by an
omission message if more exist. Long names are shortened. Gnoblin also
publishes a notice for the shell; large lists are summarized there.

An existing previous-session crash notice takes priority over warning notices.
Reloading leaves that crash notice unchanged; read warnings from the reload
result or event while it is present.

Correct the names and reload to clear the configuration notice. Validation
errors still fail the reload and keep the previous config active.

## Locking

`gnoblin.session.lock()` asks a subscribed client to show its lock UI. A client
is a shell process that subscribed to `gnoblin.session.lock-requested` on the
socket, or a Lua configuration that listens for that event with `gnoblin.on`.

The client owns the lock UI and must use Mutter's lock protocol; this Lua
method does not lock the session by itself. It takes no arguments and returns
an `Operation<LockRequest>`. `gnoblinctl session lock` makes the same request.

On success, `dispatched` is `true`. `subscribers` counts the clients
subscribed when Gnoblin targets the request. A slow connection may close before
it handles the event, so these fields do not confirm that the client showed its
UI or that the session locked. The operation fails when compositor locking is
unavailable or no client is subscribed. No Lua unlock method is provided.

A Lua listener receives a [`Request`](/shell-api/requests) of kind `"lock"`.
See [Session lock](/session-lock) for a listener that starts a locker.

The lock state is `unlocked`, `covering`, `locked`, or `failsafe`. Only
`locked` confirms the compositor's lock transition. See the
[compositor bridge](/bridge/#api-version-121-session-locking)
for socket subscription and request details.

To react to the session being idle, use an [idle watch](/shell-api/idle).
Socket clients read the idle-monitor sample with the `session.activity` call
and receive `gnoblin.session.activity-changed`; see the
[compositor bridge](/bridge/session-and-windows#session-activity).

Native sessions provide immediate reads through `gnoblin.windows`,
`gnoblin.workspaces`, `gnoblin.outputs`, `gnoblin.layers`,
`gnoblin.input.devices()`, and `gnoblin.capabilities`. See
[Lua events](/events/) for event names and callback behavior.
