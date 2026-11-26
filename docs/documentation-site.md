# Documentation site

Edit the Markdown in `docs/`. The [public site](https://gnoblin.org/) is built from those files.

## Preview

The site is in `infra/site/`. It reads the pages from `docs/`, so you edit only the Markdown. Use Node 22.17 or
later and Yarn through Corepack:

```sh
corepack enable
cd infra/site
yarn install --immutable
yarn dev
```

Open the printed localhost address. Search needs a build.

To build the static site and serve it the way the production Worker does:

```sh
yarn build
yarn preview
```

The build fails when a link in `docs/` points to a page or heading that does not exist, or when a page is in neither
the sidebar nor the unlisted list. Links to files outside `docs/` should point directly to their source on GitHub.

`yarn build` also writes the [API reference](#api-reference) and checks the [examples](#show-an-example-in-every-language).
Both need the Rust toolchain of the source repository, because the SDK generator and the SDK crates are Rust, and the reference needs
Python 3.11 or later (with `uv`, or the `venv` and `pip` modules) for its Python tool.

To check the built pages in headless Chrome, run `yarn test:pages` after `yarn build`.

## Publish

The Infrastructure workflow builds and checks the site on pull requests, which catches broken pages.
On `main` it plans the deployment. It uploads the build only in these cases:

- The repository variable `INFRA_AUTO_APPLY` is `true`.
- You run the workflow by hand with `apply` set.

Terraform uploads the build to the `gnoblin-website` Worker. See `infra/README.md` in the source repository.

## Show an example in every language

Shell pages show one example in TypeScript, Python and Rust. Put the three in a `::: tabs` container, in that order:

````text
::: tabs

```typescript
gnoblin.idle.watch(300_000, { onIdle: () => gnoblin.session.lock() });
```

```python
from gnoblin.objects.model import Gnoblin, IdleListener
...
```

```rust
use gnoblin_core::objects::ModelRoot;
...
```

:::
````

The site shows the container as tabs. Choosing a language on one example changes every example on the page, and the
site remembers the choice for the next page. The tabs are always in the same order, so a page read without scripts shows
the first one, TypeScript; that is the order of the tabs and not a preferred language.

The container follows these rules, and the build fails on a page that breaks one:

- It holds one fenced block each of `typescript`, `python` and `rust`, in that order, and nothing else.
- `::: tabs` and the closing `:::` start in the first column, and so does each fence.
- Each example is a whole file: the TypeScript and Python files you would save as `shell.ts` and `shell.py`, and the Rust
  `lib.rs` of a shell library with its `use` lines and `#[gnoblin::main]` function.

### Write for every SDK language

The SDK has three languages, and the documentation treats them alike:

- Do not write a page for one language and leave the others to a note. Every example of shell code goes in a `::: tabs`
  container with a TypeScript, a Python and a Rust example.
- Name all three spellings in prose: `isIdle` in TypeScript, `is_idle` in Python and `is_idle()` in Rust. Say where a
  language differs (for example, a Rust call returns a `Result`) beside the example, not in a footnote.
- Write headings that do not name a language, such as "Window style" and not "Window style in TypeScript".
- A page about one language (the [TypeScript](typescript-shell.md), [Python](python-shell.md) and [Rust](rust-shell.md)
  shell pages) has the same sections as the other two, in the same order: create, run, reload while it runs, types and
  editor support, imports and libraries, use the objects, window requests, troubleshooting, examples.
- Say so when something exists in one language only, and only after you have looked it up in the
  [API reference](reference/index.md) for all three. The console, for example, is a JavaScript prompt, and XR
  scripting is part of the TypeScript library.

### Check the examples

`cargo xtask docs-examples` checks every example in its own language. `yarn build` runs it, and it takes page names to
check only those: `cargo xtask docs-examples shell-api/idle.md`. Run `yarn install` in `infra/site` first, which installs
`tsc` and `pyright`.

| Language   | Checked with                                                                            | What passing means                                                |
| ---------- | --------------------------------------------------------------------------------------- | ----------------------------------------------------------------- |
| TypeScript | `tsc` in strict mode, against the declarations `gnoblin-core` and `gnoblin-std` install | Every import and member exists, and every argument has its type.  |
| Python     | `pyright` in standard mode, against the stubs `gnoblin` installs                        | The same, for modules, classes, members and arguments.            |
| Rust       | `cargo check`, in a crate that depends on `gnoblin-core` and `gnoblin-std`              | The file resolves, type-checks and borrow-checks against the SDK. |

Rust examples may use `serde_json`. The check's crate depends on it, so a Rust example can parse the JSON of a protocol the way a
project would.

The check does not start a shell, so it does not prove that an example does what the page says. Check that by running it
in a [nested session](source-development.md).

A failure names the page, the line in it and the language:

```text
docs/shell-api/idle.md:110:6: typescript: error TS2551: Property 'releas' does not exist on type 'IdleInhibitor'.
```

To keep an example on the page and out of the check, add a reason to its fence:

````text
```rust skip="needs a second output"
// The example stays on the page and out of the check.
```
````

The build prints each skipped example with its reason. Do not skip an example because it fails.

A TypeScript example that is a program outside a shell, such as a bridge client, is marked `client`. The check reads it as a Node
program, against the Node declarations (`@types/node`), and not against the shell's SDK declarations. The marker goes before any
`skip` reason:

````text
```typescript client
import { connect } from "node:net";
```
````

## API reference

The [API reference](reference/index.md) is the reference of each SDK, made by the documentation tool of that language and served as its own
HTML beside the docs:

| Language   | Tool                                            | URL                      | Reads                                                                                            |
| ---------- | ----------------------------------------------- | ------------------------ | ------------------------------------------------------------------------------------------------ |
| TypeScript | TypeDoc                                         | `/reference/typescript/` | The declarations an install gives an editor: `gnoblin-core` and `gnoblin-std`.                   |
| Python     | Sphinx with `sphinx-autoapi` and the Furo theme | `/reference/python/`     | The stub package an install gives a type checker. It reads the `.pyi` files and imports nothing. |
| Rust       | rustdoc, through `cargo doc`                    | `/reference/rust/`       | The `gnoblin-core`, `gnoblin-std` and `gnoblin-macros` crates.                                   |

`yarn build` runs `scripts/reference.mjs`, which writes the three into `infra/site/.generated/api`, and `scripts/postbuild.mjs` copies
them to `build/reference`. Git ignores both. `yarn reference` writes them again, and `node scripts/reference.mjs --only python` writes one.
A language that is missing fails the build.

- The TypeScript and Python tools read the files that an install writes for editors. `cargo xtask docs-reference` writes them into
  `infra/site/.generated/sdk`, so the reference cannot name a member the SDK lacks.
- The versions are pinned. TypeDoc is in `infra/site/package.json`, and the Python packages are in
  `infra/site/reference/python/requirements.txt`.
- The script makes the Python environment in `.generated/venv` from that file. It uses `uv` when it is installed, and `python3 -m venv`
  otherwise.
- A description is the comment above the class or member in the schema file under `crates/gnoblin-core-abi/schema` or
  `crates/gnoblin-std-abi/schema`. The generator writes it as a TSDoc comment, a Python docstring and a Rust doc comment, so it is the
  same sentence in the three references and in an editor's hover text. To change one, edit that comment. A paragraph indented by two
  spaces more than the comment around it is kept as a code block.
- The code of the SDKs has descriptions of its own beside the generated ones: the TSDoc of `data/js`, the docstrings of
  `crates/gnoblin-runtime-python/stubs` and the doc comments of `crates/gnoblin-core` and `crates/gnoblin-std`. Edit those where they are.
  The Rust build fails on a rustdoc warning (`-D warnings`), and the TypeScript build on a broken `{@link}`.
- The pages of the docs link to a reference with `/reference/typescript/`, `/reference/python/` or `/reference/rust/`, and to a type by the
  name of its file. The build checks that the language is there, and the other pages of the reference are not checked.

The references are not searched by the search box of the docs, which covers the guides. Each tool has a search of its own.

## Write for the reader

- Give each page one job. Start with what the reader can do.
- Explain unfamiliar terms before using them: a reader should not need Mutter knowledge
  to change a titlebar.
- Give complete, runnable config examples. A variable containing settings does nothing
  until a config function uses it.
- State where to put an example and what should change after applying it.
- Put the normal command before implementation details.
- Use short paragraphs and one action per numbered step.
- Explain an option beside its example: accepted values, default, units and reload.
- Keep tables short. Link to details instead of packing paragraphs into cells.
- Keep user instructions separate from protocol and testing notes.
- Capture screenshots in a fresh nested-session profile or VM and inspect them for personal data before publishing.
- Show real interface activity, without placeholder output. Put commands and explanatory text in prose or code blocks, not in staged terminal screenshots.
- Check published package names before updating installation commands.

Installation is organised around the package-first approach used by
[niri](https://niri-wm.github.io/niri/Getting-Started.html) and
[Hyprland](https://wiki.hypr.land/Getting-Started/Installation/).

Internal design notes sit beside the code they describe: `ARCHITECTURE.md` at the
root of the source repository, and a `DESIGN.md` in a crate or tool directory. They
are not part of the published user documentation.
