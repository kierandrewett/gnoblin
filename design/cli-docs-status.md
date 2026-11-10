# CLI docs status

Audit of the `gnoblinctl` examples in the user docs against a live headless session on Agent37. Log: `CliDocsE2E-logs/live-docs-run3.txt`. Drift test: `crates/gnoblinctl/tests/docs_examples.rs`.

## Verified live

| Example                                                    | Doc           | Live result                                    | rc  | Verdict |
| ---------------------------------------------------------- | ------------- | ---------------------------------------------- | --- | ------- |
| `gnoblinctl --version`                                     | gnoblinctl.md | prints client version                          | 0   | matches |
| `gnoblinctl version`                                       | gnoblinctl.md | Gnoblin and IPC versions                       | 0   | matches |
| `GNOBLIN_IPC_SOCKET=/nonexistent/ipc.sock gnoblinctl ping` | gnoblinctl.md | `cannot-connect` names the path                | 3   | matches |
| `gnoblinctl ping`, `status`, `status --json`               | gnoblinctl.md | answered; JSON keys as documented              | 0   | matches |
| `gnoblinctl output list`, `output get 6 width`             | gnoblinctl.md | object list; property value                    | 0   | matches |
| `gnoblinctl output get 6 nosuch`                           | gnoblinctl.md | usage error lists possible values              | 2   | matches |
| `gnoblinctl output call 6 set_layout`, `apply` (no params) | gnoblinctl.md | refused, names required parameters             | 2   | matches |
| `gnoblinctl window call ID nope`                           | gnoblinctl.md | clap error                                     | 2   | matches |
| `gnoblinctl session/window/workspace --help`               | gnoblinctl.md | declared signals match the doc                 | 0   | matches |
| `gnoblinctl config validate` (valid file)                  | gnoblinctl.md | `valid`                                        | 0   | matches |
| `gnoblinctl config validate` (missing file)                | gnoblinctl.md | `io: cannot read ...`                          | 1   | matches |
| `gnoblinctl --json log --limit 1`                          | gnoblinctl.md | record keys as documented                      | 0   | matches |
| `gnoblinctl log --dir /nonexistent`                        | gnoblinctl.md | exact documented message                       | 1   | matches |
| `gnoblinctl ipc dump FILE` (missing file)                  | gnoblinctl.md | error names the file                           | 1   | matches |
| `gnoblinctl ipc dump -` (stdin)                            | gnoblinctl.md | prints decoded frames                          | 0   | matches |
| `gnoblinctl completions bash`                              | gnoblinctl.md | completion script                              | 0   | matches |
| `timeout 3 gnoblinctl output watch --count 1`              | gnoblinctl.md | no signal in 3 s; prints nothing until stopped | 124 | matches |

## Checked by the drift test only

`crates/gnoblinctl/tests/docs_examples.rs` checks that every command name, flag and possible value in the listed pages exists in the live command tree. It does not run the examples. These pages are: `gnoblinctl.md`, `cli/index.md`, `troubleshooting.md`, `config.md`, `build-a-desktop.md`, `bring-your-own-shell.md`, `guides/files_and_load_order.md`, `guides/shortcuts.md`.

## Changes made

- `gnoblinctl.md` rewritten to the commands this build has. The previous text described commands that do not exist here: `input`, `animation`, `shortcut`, `lua`, `monitor`, `layer`, `focus`, `reload`, `privacy`, `logout`, `init` and `completion`.
- `output` methods are now `set_layout` and `apply`.
- `cli/index.md`, `troubleshooting.md`, `config.md`, `build-a-desktop.md`, `bring-your-own-shell.md`, `guides/files_and_load_order.md` and `guides/shortcuts.md` corrected. The shortcut-capture section in `guides/shortcuts.md` described a command that does not exist, so it was removed.

## Findings not fixed here

1. `gnoblinctl log --level WARN` shows child stderr lines that were logged at INFO. Child stderr is always logged at WARN in `crates/gnoblin-session/src/raw.rs:241` (`log_line`, `OutputStream::Stderr` arm). Stdout is logged at INFO. The level the child wrote is lost. Fix needs a design choice: parse the child's own level prefix (`is_structured`, `raw.rs:193`) or keep stderr at WARN and document it.
2. The `output watch` example needs a signal source to show output. The doc now says it prints nothing until stopped, which matches the live run.
