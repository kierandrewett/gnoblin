# Permissions and privacy

| Method                                    | Arguments                                              | Successful result                                                |
| ----------------------------------------- | ------------------------------------------------------ | ---------------------------------------------------------------- |
| `gnoblin.privacy.state()`                 | None                                                   | Read-only `PrivacyState` snapshot                                |
| `gnoblin.privacy.stop_sharing()`          | None                                                   | `Operation<{requested: integer}>`                                |
| `gnoblin.privacy.stop_recording()`        | None                                                   | `Operation<{requested: integer}>`                                |
| `gnoblin.location.authorize_app(options)` | `request_id`, `allow`, `accuracy` from a pending event | Runtime: `Operation<Result>`; `gnoblinctl lua`: read-only result |
| `gnoblin.permissions.list()`                      | None                                                   | Read-only policy, capabilities, levels, and config path          |
| `gnoblin.permissions.policy()`                    | None                                                   | Immutable policy with `default`, `rules`, and `revision`         |
| `gnoblin.permissions.check(args)`                 | `capability`, `identity`                               | Permission decision with scope details                           |
| `gnoblin.grant.list()`                            | None                                                   | `{grants = {Grant, ...}}`                                        |
| `gnoblin.grant.revoke(args)`                      | `kind`, `id`                                           | `{ok, id}`                                                       |

## Permission policy

Permission capabilities, identities, grant kinds, and scope fields use the
same values as [session permissions](/config/configure/permissions).
`gnoblinctl lua` exposes `gnoblin.permissions.list()` as a deeply read-only
snapshot.

`gnoblin.permissions.check` returns an immutable decision with `level`, `rule`,
`monitors` (a string array), `devices` (an array containing any of
`"keyboard"`, `"pointer"`, and `"touchscreen"`), `clipboard`, and `revision`.
The revision identifies the committed permission-policy snapshot.

## Privacy state

`gnoblin.privacy.state()` returns an immutable `PrivacyState` record with an
`available` field and a `revision`.

Socket clients read this state through the shared Lua runtime; see the
[compositor bridge](/compositor-bridge) for its request format.

The `available` record uses `screen_sharing`, `recording`,
`microphone_in_use`, `camera_in_use`, and `location_in_use`. Gnoblin omits an
activity field when its source is unavailable.

The native runtime reports screen-sharing and recording activity from Mutter's
tracked remote-access handles. Microphone and camera monitoring are available
when Mutter is built with remote-desktop support and can connect to PipeWire.

## Privacy controls

`gnoblinctl lua` also exposes `gnoblin.privacy.stop_sharing()` and
`gnoblin.privacy.stop_recording()`. The console waits for compositor completion
and returns a deeply read-only `{requested = integer}` result. A positive count
means Mutter was asked to stop those handles; it does not confirm that they
have closed.

It reports running audio-capture streams, including meter streams opened by
volume-control applications. A stream's self-reported application ID is not
trusted to suppress microphone activity. The monitor reports an active capture
stream; it does not inspect whether the stream is carrying audible samples.

Camera monitoring follows running PipeWire nodes whose media role is `Camera`.
It keeps the activity state for 500 ms after the last node stops to avoid
flickering. Location availability and activity come from Gnoblin's GeoClue
agent. A source is unavailable when its service or monitor cannot be reached;
unavailable does not mean inactive.

## Location requests

GeoClue must allow the `gnoblin` agent ID in its agent whitelist. On Fedora,
install the optional `gnoblin-geoclue-integration` package to add Gnoblin to
GeoClue's standard agent list:

```sh
sudo dnf install gnoblin-geoclue-integration
sudo systemctl restart geoclue.service
```

On other distributions and for source builds, append `;gnoblin` to GeoClue's
`[agent]` `whitelist`, preserving every existing ID. A file in `conf.d`
replaces that value. Include all IDs you want to retain in the drop-in.

Restart GeoClue with your distribution's service manager, then run
`gnoblinctl privacy`. The location source should report `inactive` when unused.
The `unavailable` status means GeoClue is unreachable or rejected Gnoblin's
registration.

Gnoblin publishes `gnoblin.location.authorization-requested` when GeoClue asks
whether an application may use location. A shell or Lua handler can answer
with `gnoblin.location.authorize_app`:

```lua
gnoblin.on("gnoblin.location.authorization-requested", function(event)
  gnoblin.location.authorize_app {
    request_id = event.request_id,
    allow = false,
    accuracy = 0,
  }
end)
```

This handler denies every request. A shell that asks the user for consent can
return `allow = true` with the selected accuracy after the user approves.
The [compositor bridge](/compositor-bridge) describes how a socket client
receives and answers these requests. GeoClue configuration that does not allow
the agent must be fixed by the system administrator before the capability
becomes available.

GeoClue supplies `app_id` as the application's desktop ID. Treat it as a
request attribute, not an authenticated identity.

Accuracy levels are:

- `0`: none. Use this when denying a request.
- `1`: country.
- `4`: city.
- `5`: neighborhood.
- `6`: street.
- `8`: exact.

An allowed answer must be nonzero, no more precise than the request, and
within the global [location policy](/config/configure/location). A disabled
policy denies the request.

The event fields are `request_id`, `app_id`, `requested_accuracy`, and
`expires_at_us`, plus the standard event metadata.

Requests expire after 25 seconds. Gnoblin denies unanswered requests and
requests still pending when the runtime stops or the GeoClue agent becomes
unavailable. The answer is a one-use operation. Socket clients must answer on the same
connection that received the request event.

`gnoblin.privacy.stop_sharing()` requests closure of tracked non-recording
handles. `gnoblin.privacy.stop_recording()` requests closure of tracked
recording handles. Each operation result contains integer `requested`, the
number of handles passed to `meta_remote_access_handle_stop()`. A positive
count confirms that the calls were issued, not that a session has already
closed. Mutter reports closure later through `gnoblin.privacy.changed`; these
methods do not revoke a saved portal grant.

## Permission snapshots

`gnoblin.permissions.policy()` returns the committed Gnoblin policy. The record has a
default level, ordered rules, and revision. Subscribe to
`gnoblin.permission.changed` to observe committed changes.

`gnoblin.permissions.list()` returns that policy with capability names, supported
levels, and the configuration path.

## Portal grants

Use `gnoblin.grant.revoke {kind, id}` with the `kind` and `id` from a listed
portal grant. The optional `created_at` timestamp rejects a stale record if a
new grant reuses its ID. `gnoblinctl lua` exposes this operation and returns a
deeply read-only result after the compositor confirms completion.

Each grant record contains:

| Field           | Type and values                                                  |
| --------------- | ---------------------------------------------------------------- |
| `id`            | Opaque string returned by `gnoblin.grant.list()`.                        |
| `kind`          | `"screen-cast"` or `"remote-desktop"`.                           |
| `requester`     | Verified portal identity, such as `app-id:org.example.Recorder`. |
| `devices`       | Integer bitmask: keyboard `1`, pointer `2`, touchscreen `4`.     |
| `clipboard`     | Boolean.                                                         |
| `screenStreams` | Boolean indicating whether a screen-stream selection is stored.  |

Both methods return `Operation` handles. Grant listing and revocation run in
the portal backend, which validates the stored record before returning or
removing it. A missing grant, invalid record, or unavailable backend fails the
operation.

The native Lua runtime also provides `gnoblin.portals.grants(filter?)` for an
immediate snapshot. Pass `{kind = "screen-cast"}` or
`{kind = "remote-desktop"}` to filter the records. Each immutable
`PortalGrant` record contains:

| Field                | Type and values                                                 |
| -------------------- | --------------------------------------------------------------- |
| `id`                 | Opaque portal grant ID.                                         |
| `kind`               | `"screen-cast"` or `"remote-desktop"`.                          |
| `requester`          | Verified portal identity.                                       |
| `devices`            | Array of `"keyboard"`, `"pointer"`, or `"touchscreen"`.         |
| `clipboard`          | Boolean.                                                        |
| `has_screen_streams` | Boolean indicating whether a screen-stream selection is stored. |
| `created_at`         | Unix timestamp in milliseconds.                                 |
| `revision`           | Revision of the current native grant snapshot.                  |

Call `grant:revoke()` on a record from this snapshot. It returns an
`Operation` whose successful value contains `ok` and `id`. The portal backend
checks that the record still has the same creation time before revoking it.
Stale records fail.

New grants store their creation time. Older grants use file modification time
at whole-second precision; this is inferred metadata, not a verified consent
time. The collection is available after the native portal grant snapshot has
loaded.

Subscribe to `gnoblin.portal.grant-added` and
`gnoblin.portal.grant-removed` to track changes. The [compositor bridge](/compositor-bridge)
documents the socket methods and event subscriptions.
