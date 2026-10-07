# Session and runtime health

`gnoblin.session.status()` returns a record with these fields:

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
`gnoblin.session.lock-state-changed` for lock transitions.

`gnoblin.session.logout()` takes no arguments and returns an operation whose
successful value is `{accepted = true}`. After the compositor confirms that
operation, the supervisor exits successfully and stops the compositor. The
session wrapper then stops Gnoblin's user services and returns to the login
manager. A socket caller may receive EOF while the session is closing.

`gnoblinctl lua` exposes this operation and `gnoblin.session.lock()` too. The
lock result reports that a subscribed shell client received the request; check
`session.status()` to confirm the compositor entered a locked state.

The [compositor bridge](/compositor-bridge) documents the socket request format.

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
gnoblin.events.on("gnoblin.runtime.status-changed", function(event)
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

After the commit, the runtime also emits `gnoblin.focus.policy-changed` and
`gnoblin.permission.changed` when those effective policies changed. These
events carry the committed policy snapshot and configuration revision. A
failed reload emits `gnoblin.config.reload-failed` while the previous runtime
remains active.

## Locking

`gnoblin.session.lock()` asks a subscribed external shell client to show its
lock UI. The client owns that UI and must use Mutter's lock protocol; this Lua
method does not lock the session by itself. It takes no arguments and returns
an `Operation<LockRequest>`.

On success, `dispatched` is `true`. `subscribers` counts connected clients
subscribed when Gnoblin targets the request. A slow connection may close before
it handles the event, so these fields do not confirm that the client showed its
UI or that the session locked. The operation fails when compositor locking is
unavailable or no client is subscribed. No Lua unlock method is provided.

The lock state is `unlocked`, `covering`, `locked`, or `failsafe`. Only
`locked` confirms the compositor's lock transition. See the
[compositor bridge](/compositor-bridge#api-version-121-session-locking)
for socket subscription and request details.

`gnoblin.session.activity()` returns the latest native idle-monitor sample.
Its fields are `available`, `idle`, `threshold_ms`, `idle_for_ms`, and
`revision`. Gnoblin uses a fixed threshold of 120 seconds. Idle inhibitors and
desktop idle-timeout preferences do not change it.

When the sample is available and idle, `idle_for_ms` advances from the last
sample using the supervisor's monotonic clock. If monitoring is unavailable,
`idle` is false and `idle_for_ms` is zero.

`gnoblin.session.activity-changed` is emitted when availability or idle state
changes. Its `idle_for_ms` is sampled at that transition and does not update
continuously. The event also carries `threshold_ms`, `revision`, `sequence`,
and monotonic-clock `time`.

Native sessions provide immediate reads through `gnoblin.windows`,
`gnoblin.workspaces`, `gnoblin.monitors`, `gnoblin.layers`,
`gnoblin.input.devices()`, and `gnoblin.capabilities`. See
[Lua events](/config/lua-events) for event names and callback behavior.
