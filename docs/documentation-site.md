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

`yarn build` also writes the data of the [API reference](#api-reference) and checks the [examples](#show-an-example-in-every-language).
Both need the Rust toolchain of the source repository, because the SDK generator and the SDK crates are Rust.

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
  [API reference](/reference) for all three. The console, for example, is a JavaScript prompt, and XR
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

The [API reference](/reference) is made by this site, from one file the SDK generator writes, `reference.json`. It has a page for each
class and service (`/reference/core/<class>` and `/reference/std/<class>`), one page of types for each area (`/reference/core/types/<area>`),
and one page of error codes (`/reference/errors`). A reader picks TypeScript, Python or Rust once, and every name, signature and example on
the page follows.

| What                                  | URL                                                  |
| ------------------------------------- | ---------------------------------------------------- |
| The index of both libraries           | `/reference`                                         |
| A class or a service                  | `/reference/core/client`, `/reference/std/workspace` |
| Records, enums, interfaces, callbacks | `/reference/core/types/client`                       |
| The error codes                       | `/reference/errors`                                  |
| A property or a method                | `#app_id`, `#focus` (the schema's snake case name)   |
| An event                              | `#event-title_changed`                               |

`yarn build` runs `scripts/reference.mjs`. It runs `cargo xtask gen-sdks --target reference` and copies the file that command writes to
`infra/site/.generated/reference.json`, which Git ignores. `yarn reference` writes it again.

On a machine that cannot build the SDK crates, `node scripts/reference.mjs --from FILE` copies a file that was generated elsewhere.

- A name, a type and a declaration is what the generator wrote for that language, the line the SDK file has: the `.d.ts` declaration for
  TypeScript, the `.pyi` stub for Python and the generated API for Rust. The site spells nothing itself, so the reference cannot name a member
  the SDK lacks.
- A description is the comment above the class or member in the schema file under `crates/gnoblin-core-abi/schema` or
  `crates/gnoblin-std-abi/schema`, so it is the same sentence in the three languages and in an editor's hover text. To change one, edit that
  comment. A name in backticks in a description follows the language that is read. A paragraph indented by two spaces more than the comment
  around it is kept as a code block.
- The keys `title`, `range`, `unit`, `default`, `raises` and `example` of the schema files are shown as the area's name, a range line, a unit,
  a default line, the Errors table of a member and its Example. The text of each error code is the table of
  [Operations, errors and timers](shell-api/errors.md#error-codes); the build fails on a code that table lacks.
- The picker uses the language of the guides' code tabs: the `gnoblin-language` key of `localStorage`, `data-lang` on `<html>` and `?lang=`
  in a link. It changes spelling and nothing else: the anchors and the URLs are the same in the three languages.
- Search finds a class or a member by any of its three spellings (`titleChanged`, `title_changed` and `on_title_changed`), as an
  "API reference" result, and shows it in the language that is picked.
- The old `/reference/typescript/`, `/reference/python/` and `/reference/rust/` redirect to `/reference`.
- A page of the docs links to a reference page by its URL, `/reference/core/client`; the build checks that the page exists.

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
