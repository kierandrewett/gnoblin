# Formatting and lint

Run checks from the repository root. Tools are pinned in
`.pre-commit-config.yaml`; `scripts/quality.sh` manages pre-commit.

## Check or format

```sh
just lint
just format --files docs/configuration.md
```

`lint` is read-only. `format` rewrites files.
With no file arguments, they select all tracked files; use `--files` for new
files or when unrelated edits are in progress.

## Prerequisites

Install Python 3 and [uv](https://docs.astral.sh/uv/getting-started/installation/).
The first run downloads tools; later runs reuse the cache.

| Files | Additional requirement                                 |
| ----- | ------------------------------------------------------ |
| QML   | Qt 6 QML tools; set `QMLFORMAT` for a nonstandard path |
| Rust  | `rustfmt`                                              |
| Nix   | Nix with flakes                                        |

Missing tools fail the check. They are not silently skipped.

## Common wrapper

```sh
./scripts/quality.sh lint --files path/to/file.py
./scripts/quality.sh format --files path/to/file.py
./scripts/quality.sh lint --from-ref origin/main --to-ref HEAD
```

The same wrapper works in Bingux. Its Make targets are `make lint` and
`make format ARGS="--files path/to/file"`.

## What is checked?

| Language                        | Tools                |
| ------------------------------- | -------------------- |
| Python                          | Ruff                 |
| Shell                           | ShellCheck, shfmt    |
| JavaScript                      | ESLint, Prettier     |
| QML                             | Qt parser, qmlformat |
| C/C++                           | clang-format         |
| Rust                            | rustfmt              |
| Lua                             | StyLua               |
| Nix                             | nixfmt               |
| Markdown, YAML, JSON, CSS, HTML | Prettier             |

Submodules, patches, locks and generated output are excluded.
QML checks do not replace `qmllint` with the matching installed modules.
Builds and runtime tests remain separate.

EditorConfig records layout conventions. Keep shared tooling identical in
Gnoblin and Bingux. Test tooling changes with `python3 tests/quality-tools.py`.

## Optional commit hook

```sh
uv tool run --from pre-commit==4.6.2 pre-commit install
```

This checks staged files without formatting. Use `uninstall` to remove it.
Tool setup does not install hooks automatically.
