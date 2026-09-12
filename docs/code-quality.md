# Code quality

Gnoblin and Bingux use the same pre-commit configuration. Run it through
`scripts/quality.sh` from either checkout. Tools are pinned in
`.pre-commit-config.yaml`; pre-commit is pinned in the wrapper.

## Setup

Install [uv](https://docs.astral.sh/uv/getting-started/installation/) and Python 3.
The first run downloads the pinned tools into pre-commit's cache. It needs
network access. Later runs reuse that cache. No project Python environment or
Node package manifest is needed.

Install Qt 6 QML tools for QML files. The wrapper finds `qmlformat` on PATH or in
`/usr/lib64/qt6/bin` and `/usr/lib/qt6/bin`. Set `QMLFORMAT=/path/to/qmlformat`
when Qt is installed elsewhere. Use the Qt version used to develop the shell;
this setup was tested with Qt 6.10.3. Formatting does not sort imports or
normalise property order.

Rust files need `rustfmt` (`rustup component add rustfmt`). The Rust packages
use edition 2024. Nix files need Nix with flakes enabled. The hook obtains
nixfmt 1.3.1 from a pinned Nixpkgs revision and reuses the Nix store on later
runs. It clears `LD_LIBRARY_PATH` for this command so the desktop's Qt and
Mutter libraries cannot override the formatter's Nix libraries.

A missing tool is an error. Checks do not silently skip a language.

## Commands

From Gnoblin:

```sh
just lint
just format --files src/config/gnoblin-config.c
```

From Bingux:

```sh
make lint
make format ARGS="--files shell/bingux/Theme.qml"
```

The common interface works in both repos, including paths with spaces:

```sh
./scripts/quality.sh lint
./scripts/quality.sh lint --files path/to/file.py
./scripts/quality.sh format --files path/to/file.py
./scripts/quality.sh format
```

`lint` is read-only. `format` rewrites files. With no arguments, both commands
select all tracked files. Use `--files` to include new files before staging.
The tools exclude upstream submodules, patch files, dependency locks and build
output. Git also excludes untracked and ignored output from `--all-files`.
Format selected files while either checkout has work in progress. Review the
diff after formatting.

Run both checkouts from their parent directory:

```sh
(cd gnoblin && ./scripts/quality.sh lint)
(cd bingux && ./scripts/quality.sh lint)
```

To check changed files against a branch, pass pre-commit's revision options:

```sh
./scripts/quality.sh lint --from-ref origin/main --to-ref HEAD
```

Optional commit checks can be installed explicitly:

```sh
uv tool run --from pre-commit==4.6.2 pre-commit install
```

This checks staged files without formatting them. To remove the hook, use the
same command with `uninstall`. Tool setup does not install Git hooks itself.

## Coverage

| Files                                                   | Checks                         | Formatter    |
| ------------------------------------------------------- | ------------------------------ | ------------ |
| Python, including executable scripts without extensions | Ruff error rules               | Ruff         |
| POSIX and Bash scripts, including executable scripts    | ShellCheck warnings and errors | shfmt        |
| JavaScript, MJS and CJS                                 | ESLint correctness rules       | Prettier     |
| QML                                                     | Qt parser and formatting check | qmlformat    |
| C and C++                                               | Formatting check               | clang-format |
| Rust                                                    | Formatting check               | rustfmt      |
| Lua                                                     | Parser and formatting check    | StyLua       |
| Nix                                                     | Parser and formatting check    | nixfmt       |
| JSON, YAML, Markdown, CSS, SCSS and HTML                | Parser and formatting check    | Prettier     |

Qt `.pragma` and `.import` lines in JavaScript are preserved. The adapter masks
these lines only while passing the source to Prettier or ESLint. QML test
fragments (`*.inc.qml`) are formatted inside a temporary root object; the
wrapper is removed before the fragment is written. ESLint avoids
undefined-global rules because GJS and QML supply runtime globals.

C/C++ keeps Gnoblin's existing 100-column style. Other configurable formatters
use 120 columns and four spaces, with two spaces for JSON and YAML. Rust uses
standard rustfmt. Qt and Nix use their native layout rules. EditorConfig records
the common editor settings and preserves Makefile tabs.

These checks do not replace native builds, Rust tests or runtime tests.
`qmllint` needs the installed Quickshell modules and matching Qt import paths;
the portable QML check only parses and formats. XML protocols, Meson files,
packaging specs, shader sources, generated C include data and other build data
retain their existing build-specific validation.

Whole-repository lint is expected to pass before committing. Fix findings in
focused changes. Necessary import-order exceptions have local comments; there
is no suppressed error baseline.

Keep the configuration and adapter files identical in both repos. Each checkout
is self-contained and does not depend on the other being present.

## Tooling regression test

Run `python3 tests/quality-tools.py`. It uses a temporary Git repository to
check each language, read-only behaviour, repeated formatting, Qt directive
preservation, extensionless scripts, paths with spaces and excluded output.
It also checks that a QML parser failure leaves the original file intact.
