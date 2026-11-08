# Config apply status

This audit records which configuration sections take effect when a document is
committed, and which only take effect when the session starts. It was taken from
the code on `rust-compositor` at commit `4539251f4`.

## How a change reaches the session

There are two paths.

- **Start.** `crates/gnoblin/src/main.rs` calls each `configure_*_from_environment`
  before `Compositor::run`. Each one reads its section from the last accepted
  document and writes it into the session config. Anything read here is fixed for
  the life of the process.
- **Commit.** `crates/gnoblin-compositor/src/ipc/server.rs` `commit()` handles a
  `config.commit` from the runtime. It saves the document as last good, publishes
  the settings object, and then applies a short list of values to the running
  session. Nothing else is re-read.

The commit path applies these values live:

| Section         | Key              | Live effect                                                                                                |
| --------------- | ---------------- | ---------------------------------------------------------------------------------------------------------- |
| `appearance`    | `reduced_motion` | Animations turn off or on in the compositor loop.                                                          |
| `appearance`    | `color_scheme`   | Portal dialog scheme (`portal.configure`).                                                                 |
| `monitors`      | `global_scale`   | Outputs still at the identity scale are set to it (`apply_global_scale`).                                  |
| `window_rules`  | all              | Rules replaced (`control.set_window_rules`). A rule the session cannot carry out refuses the whole commit. |
| settings object | all              | Published to subscribers (`store.publish_settings`).                                                       |

Every other section is read only at start.

## Status by section

Read-at-start means the value is taken from the document when the session
starts. A commit does not change it.

| Section                       | Read at            | Applies on commit | Notes                                                                                                                                                                   |
| ----------------------------- | ------------------ | ----------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `layouts`                     | start              | no                | `configure_layouts_from_environment` in `main.rs`.                                                                                                                      |
| `cursor`                      | start              | no                | `theme`, `size`, and `providers` are fixed for the process.                                                                                                             |
| `bell`                        | start              | no                | `visual_bell` settings are copied into the Wayland server config once.                                                                                                  |
| `accessibility`               | start              | no                | Read by `configure_accessibility_from_environment`.                                                                                                                     |
| `window_management`           | start              | no                | Only `window_rules` is live, and it is handled separately above.                                                                                                        |
| `input` (keyboard, xkb)       | start              | no                | Copied into `config.wayland.keyboard` by `session_tables`.                                                                                                              |
| `input_sources`               | start              | no                | Copied into `config.wayland.input_sources` by `session_tables`.                                                                                                         |
| `monitors` (`global_scale`)   | start, then commit | yes               | See above.                                                                                                                                                              |
| `monitors` (`privacy_screen`) | start              | no                | Parsed into `MonitorSettings`, but nothing in the session reads it. See the gap below.                                                                                  |
| `xwayland`                    | start              | no                |                                                                                                                                                                         |
| `renderer`                    | start              | no                |                                                                                                                                                                         |
| `extensions`                  | start              | no                | A refused extension is logged and left out; the session starts without it.                                                                                              |
| `keyring` and `prompts`       | services start     | no                | The services role reads the saved last-good file before the runtime commits anything. A commit reaches it only after a restart. See `docs/config/configure/keyring.md`. |

## Gaps

- **`monitors.privacy_screen` is not applied.** The value is parsed into
  `MonitorSettings.privacy_screen` (`monitors.rs:63`). No code in
  `gnoblin-wayland`, `gnoblin-backend*`, or `gnoblin-compositor` reads it, so a
  document that sets it changes nothing, at start or on commit. Either wire it to
  the backend or remove the key from the schema. Not fixed in this change.
- **Startup-only sections are silent.** A user who edits `cursor`, `bell`, or
  `input` and commits sees the commit accepted, and nothing changes. The docs for
  each section must say so. See the "When a change applies" sections.

## Next

Each startup-only section needs a live path or an honest "restart to apply" in its
docs. The order follows user impact: input layout and repeat, then the cursor,
the accessibility visual state, and the bell. Each takes a failing test first on an
empty last-good document.
