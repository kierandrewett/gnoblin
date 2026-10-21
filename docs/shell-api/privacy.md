# Privacy

| Method                                    | Arguments                                              | Successful result                                                |
| ----------------------------------------- | ------------------------------------------------------ | ---------------------------------------------------------------- |
| `gnoblin.privacy.state()`                 | None                                                   | Read-only `PrivacyState` snapshot                                |
| `gnoblin.privacy.stop_sharing()`          | None                                                   | `Operation<{requested: integer}>`                                |
| `gnoblin.privacy.stop_recording()`        | None                                                   | `Operation<{requested: integer}>`                                |
| `gnoblin.location.authorize_app(options)` | `request_id`, `allow`, `accuracy` from a pending event | Runtime: `Operation<Result>`; `gnoblinctl lua`: read-only result |

## Privacy state

`gnoblin.privacy.state()` returns an immutable `PrivacyState` record with an
`available` field and a `revision`.

Socket clients read this state through the shared Lua runtime; see the
[compositor bridge](/bridge/) for its request format.

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
The [compositor bridge](/bridge/) describes how a socket client
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
closed. Mutter reports closure later through `gnoblin.privacy.changed`.
