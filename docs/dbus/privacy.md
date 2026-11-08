# Privacy Interface

Use `org.gnoblin.Privacy` to learn whether a microphone or camera is in use right now.

Gnoblin exports this interface on the session bus as `org.gnoblin.Privacy`, at `/org/gnoblin/Privacy`. The state comes from the PipeWire graph: a running capture node for the microphone or the camera counts as in use.

## Methods

| Member        | Arguments | Result                                                       | Behavior                                                                              |
| ------------- | --------- | ------------------------------------------------------------ | ------------------------------------------------------------------------------------- |
| `GetActivity` | None      | `(available: boolean, microphone: boolean, camera: boolean)` | Returns the last published state. Every value is `false` until PipeWire has answered. |

`available` is `true` only while the PipeWire graph has been read on the current connection. When it is `false`, the two use flags are also `false`, and they do not mean that nothing is in use.

## Signal

| Signal            | Arguments                                                                    | Behavior                                                                                           |
| ----------------- | ---------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------- |
| `ActivityChanged` | `available: boolean`, `microphone_in_use: boolean`, `camera_in_use: boolean` | Emitted once each time one of the three values changes. A repeat of the same state is not emitted. |

## Failure modes

- **No session bus, or the name is taken.** The interface is not served and nothing is published, rather than answering an activity nobody observed. Each cause is logged once.
- **PipeWire is not running.** The state stays all-false. The service retries every two seconds and logs the retry once per outage.
- **PipeWire goes away after it was read.** The connection is rebuilt and the state drops to unavailable. The change is emitted as an `ActivityChanged` signal.
- **A capture node is removed or stops running.** Its activity is cleared. A node that reports an error drops only itself.

## Test coverage

The state rules (which nodes count, removal, errors, connection loss, reset, and emitting only real changes) are unit-tested in `crates/gnoblin-service-privacy/src/model.rs`, with no bus and no PipeWire. The bus layer and the PipeWire thread have no test yet, and the interface has not been run against a real PipeWire daemon.
