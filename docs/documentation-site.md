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
site remembers the choice for the next page. Without scripts the page shows TypeScript.

The container follows these rules, and the build fails on a page that breaks one:

- It holds one fenced block each of `typescript`, `python` and `rust`, in that order, and nothing else.
- `::: tabs` and the closing `:::` start in the first column, and so does each fence.
- Each example is a whole file: the TypeScript and Python files you would save as `shell.ts` and `shell.py`, and the Rust
  `lib.rs` of a shell library with its `use` lines and `#[gnoblin::main]` function.

### Check the examples

`cargo xtask docs-examples` checks every example in its own language. `yarn build` runs it, and it takes page names to
check only those: `cargo xtask docs-examples shell-api/idle.md`. Run `yarn install` in `infra/site` first, which installs
`tsc` and `pyright`.

| Language   | Checked with                                                                            | What passing means                                                |
| ---------- | --------------------------------------------------------------------------------------- | ----------------------------------------------------------------- |
| TypeScript | `tsc` in strict mode, against the declarations `gnoblin-core` and `gnoblin-std` install | Every import and member exists, and every argument has its type.  |
| Python     | `pyright` in standard mode, against the stubs `gnoblin` installs                        | The same, for modules, classes, members and arguments.            |
| Rust       | `cargo check`, in a crate that depends on `gnoblin-core` and `gnoblin-std`              | The file resolves, type-checks and borrow-checks against the SDK. |

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

## API reference

The pages under [API reference](reference/index.md) are not in `docs/`. `yarn build` runs `cargo xtask docs-reference`,
which writes them into `infra/site/.generated/docs` from the schema catalog, and the site reads them with the pages of
`docs/`. Git ignores them. `yarn reference` writes them again.

- Each page lists every type of one library for one language. The declarations are the ones the type checker reads.
- A description is the comment above the class or member in the schema file under `crates/gnoblin-core-abi/schema` or
  `crates/gnoblin-std-abi/schema`. To change one, edit that comment.
- A page names each language's own spelling: `isIdle` in TypeScript, `is_idle` in Python and `is_idle()` in Rust.

Link to a type from a page with `/reference/typescript/core#idlewatch`, and to the pages of another language the same way.

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
