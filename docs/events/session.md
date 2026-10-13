# Session and Permission Events

Observe lock state, session activity, permission decisions, and portal grant changes.

Session and permission events are available in a running Gnoblin session when their source service is available.

### Session state

`gnoblin.session.lock-state-changed` reports these session lock states:

- `unlocked`: no active session lock.
- `covering`: the compositor has started the lock transition.
- `locked`: Mutter confirmed the lock transition.
- `failsafe`: the lock client failed and Mutter retained its failsafe state.

Only `locked` confirms the lock transition. A `session.lock()` operation result
reports request dispatch and does not confirm delivery or lock state.

`gnoblin.privacy.changed` reports available screen-sharing, microphone, camera,
and location activity in `state`. Activity appears only for available sources.
The native runtime monitors screen sharing, PipeWire microphone and camera
activity, and GeoClue location activity. GeoClue authorization requests arrive
through `gnoblin.location.authorization-requested`; answer them with
`gnoblin.location.authorize_app`. See [Permissions and privacy](/shell-api/permissions-privacy)
for accuracy levels, deadlines, and the authorization rules.

`gnoblin.capability.changed` fires when a monitored native capability becomes
available or unavailable. Gnoblin updates the capability snapshot first.
Unavailable monitor capabilities report one of these reasons:

- `remote_desktop_disabled`: Mutter was built without remote-desktop support.
- `pipewire_unavailable`: the monitor is disconnected or could not connect.

Pass `focus_context` to `Window:focus(context)` to use the authority granted to
that event. Lua cannot inspect or create the protected value.

### Portal events

Gnoblin updates the portal-grant snapshot before dispatching either grant
event. Events include a revision, sequence, and monotonic time.

- `gnoblin.portal.grant-added` carries a read-only `PortalGrant` snapshot.
- `gnoblin.portal.grant-removed` carries the opaque grant ID and portal kind.

`gnoblin.permission.changed` fires after a successful config commit changes the
portal policy. It carries the committed policy and event metadata.

The policy contains a default level, ordered rules, and the committed settings
revision. Its revision matches the event's top-level revision. Rejected or
unchanged policies do not emit this event.

The compositor socket and its directory are accessible only to the current
user. Any process running as that user can subscribe. Treat same-user socket
clients as trusted shell components. See the
[compositor bridge](/bridge/shortcuts-and-events#api-version-110-shortcut-focus-grants)
for the request format.
