# Capture API

Status: design. Phase 1 (still images) is the only phase to build. Phase 2 (streams) is design only and is not enabled for any role.

## Purpose

Gnoblin lets trusted parts of the session take still pictures of windows, outputs and regions.

Today the portal role can take an output picture through `capture.screen`, which uses the `CaptureScreen` capability. That capability was granted for portal screenshots after user consent. This document adds a separate path for trusted callers. It sets who may capture, what they may capture, and which checks run first.

Out of scope: captures by extensions, captures by ordinary clients, streams, and any change to portal screen sharing.

## Targets

Each `capture.still` request names exactly one target.

- Window: the pixels in the window's own buffer, cropped to its content rectangle. Parts covered by other windows are included, because the client drew them. Popups are excluded. A window with no drawn buffer (unmapped, or minimised) is refused as `window-not-drawn`.
- Output: the composed picture of one output, as the backend screenshot returns it.
- Region: a rectangle of one output, cropped from the output picture.

The compositor never writes files. Phase 1 returns the PNG bytes in the reply. The client decides where the file goes.

## Authorization and consent contract

### Capability

- New capability `capture-window` (`Capability::CaptureWindow`). It is the handshake name.
- Granted to the `runtime` role only (`grants("runtime")`).
- Not granted to: `services`, `portal`, `input`, any extension, or any ordinary client.
- Not granted to the session owner's ordinary-socket connections. `owner_grants` does not add it. A same-uid program that reaches the ordinary socket never gets `capture-window`.
- The portal keeps `CaptureScreen`. `CaptureScreen` does not authorize `capture.still`. `capture-window` does not authorize `capture.screen`. No window target reuses `CaptureScreen`.

### Who may call `capture.still`

A connection may call `capture.still` only if it holds `capture-window`. Any other connection is refused `denied` before the graph sees the call.

A user shell that needs a capture asks the runtime. It does not hold the capability itself.

### What may be captured

- Windows of other clients: only through a `capture-window` connection.
- Outputs and regions: only through a `capture-window` connection.
- No request may name "all windows" or "all outputs". Each request names one target.

### Consent

| Target kind                                       | User prompt                                                             | Audit record            | Privacy indicator                                               |
| ------------------------------------------------- | ----------------------------------------------------------------------- | ----------------------- | --------------------------------------------------------------- |
| Window, output, region (still), caller is runtime | Not required                                                            | Required, every capture | Required, signal before pixels are read (see Privacy indicator) |
| Any still, caller is not runtime                  | Not applicable: no such caller exists. Any future caller needs a prompt | Required                | Required                                                        |
| Stream (phase 2)                                  | Required, each stream                                                   | Required                | Required for the stream's lifetime                              |

Why no prompt for the runtime: the runtime is the trusted process that evaluates the user's configuration. Nothing outside the session can reach its connection. The audit record and the indicator keep the capture visible.

### Audit record

- Phase 1: the compositor writes one structured log line for each `capture.still` call. The line holds the caller role, the target kind, the target id, the size, and the outcome (`ok` or the refusal code).
- The full audit belongs to the request interceptor. Unverified: the request-intercept design is not in this tree yet, so the log line is the only record until it lands.

## Locked session refuses

- While the session lock is up, or the lock embargoes capture, `capture.still` refuses every target kind with `locked`.
- The check runs before any pixel is read. It uses the same observation snapshot as `capture.screen` (`still::choose_output`).
- A window target never returns lock-cover pixels. While locked, no window target is readable.
- A View capture (`gnoblin_view_source_manager_v1`, ext-image-copy-capture) is embargoed by the same lock: a granted View stream is refused while the lock is up, and a running one stops and does not resume.

## Secure treatment

- The `capture.screen` handler comment says the backend screenshot applies the configured secure treatment to secure layers. Unverified: the backend code that does this was not read in this design pass.
- No per-window secure flag was found in the code. The layer design (`design/layers.md`) has a `secure` field, but it applies to layers, not to windows.
- Window stills read the client buffer directly. The output masking path does not run for them. So a window still of secure content is not masked today.
- Required rule: secure content never reaches a `capture.still` reply. Phase 1 must not ship window stills until the Needs Kieran question on secure windows is answered.

### Magnifier

- The magnifier lives in `crates/gnoblin-a11y`. Its zoom is a view effect.
- Captures show the unzoomed desktop. Window stills read client buffers, so the zoom does not touch them.
- Unverified: whether the output screenshot path includes the magnifier zoom. If it does, the output path needs a fix before phase 1 ships.

## Privacy indicator

- The design names `org.gnoblin.Privacy` as the privacy surface (`design/e2e-privacy.md`). Its payload names no app (`design/e2e-privacy.md`, the note on indicator payloads).
- Required rule: while a capture runs, the indicator shows active capture.
- Unproven: a still is one instant, so the signal must come before the pixels are read. Unverified: whether `org.gnoblin.Privacy` reports still captures at all. Today its monitor covers PipeWire nodes.
- Phase 1 cannot claim the indicator works until the active-still signal is shown in a test.

## Streams (phase 2, design only)

Streams are not enabled for any role. Nothing in this section is built.

Before a stream may start, all of these must be designed and agreed:

- Consent: a user prompt for each stream, because a stream runs over time.
- Capability: either `capture-window` with a stream member, or a new capability. The portal ScreenCast path (`crates/gnoblin-pipewire-export`) uses the consent grant of that path. It must not be reused for `capture-window`.
- Indicator: `org.gnoblin.Privacy` stays active for the stream's whole lifetime.
- Lock: the lock stops every stream, as the export already does for ScreenCast sessions.
- Node ids: each node id belongs to the connection that opened it.

## Needs Kieran

1. Secure windows: is a per-window secure flag wanted? Should window stills of secure content be refused, or masked?
2. Privacy indicator: should `org.gnoblin.Privacy` show still captures, or only streams?
3. Audit: may the phase 1 log line stand in for the request-intercept audit until that design lands?
4. Owner ordinary socket: confirm that same-uid programs never get `capture-window`. This document says they never do.
5. Phase 2: which party answers the stream prompt, and what does the prompt say?
