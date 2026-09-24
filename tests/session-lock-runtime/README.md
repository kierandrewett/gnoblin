# ext-session-lock-v1 runtime smoke suite

`session-lock-smoke.c` is a deliberately tiny Wayland client. It renders one
opaque shared-memory buffer after the first `configure`, then observes the
standard `ext-session-lock-v1` events. It has no authentication code and must
never be installed as a locker.

`run-nested.sh` builds it from Gnoblin's vendored protocol XML and starts a
private headless Gnoblin session. It checks the security-relevant protocol
sequence:

1. a lock surface is configured, acknowledged, and committed, and the
   compositor eventually reports `locked` after a protected presentation;
2. under Gnoblin's frame-deferred policy, destroying an unconfirmed lock
   object leaves no stuck lock and a new owner can acquire the protocol;
3. a concurrent second lock receives `finished`;
4. an owner may unlock only after `locked` and the server processes the request;
5. killing the owner does not unlock the compositor, and a policy-supported
   replacement can take over.

The runner exits 77 (and prints `SKIP`) when the installed Gnoblin has no
advertised `ext_session_lock_manager_v1`. That is expected while the protocol
is intentionally fail-closed. A skipped run proves only that the test harness
compiled; it is not runtime evidence for session locking.

Run it after installing a build which advertises the protocol:

```sh
tests/session-lock-runtime/run-nested.sh
```

The test client reports whether `locked` arrived before its own first buffer
was committed. Both results can conform: the protocol permits the compositor
to confirm a presented opaque fallback frame. The `locked` event itself is the
protocol's presentation guarantee; client-side Wayland callbacks cannot prove
which compositor frame was shown.

The pre-`locked` destroy case is a Gnoblin policy test, not a generic protocol
requirement. The protocol allows a compositor to send `locked` immediately
after `lock` when it already has a presented opaque fallback; a client which
pipelined `lock; destroy` would then be invalid. Gnoblin deliberately defers
that event to its presentation path, making cancellation testable here.

The test client is useful for any compatible locker implementation: it uses
only the public Wayland protocol and makes no Gnoblin-specific D-Bus calls.
