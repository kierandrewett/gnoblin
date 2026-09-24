# Documentation site

Edit the Markdown in `docs/`. VitePress builds the
[public site](https://kierandrewett.github.io/gnoblin/) from those files.

## Preview

From the repository root:

```sh
npm ci
npm run docs:dev
```

Open the printed localhost address. To check links and build static HTML:

```sh
npm run docs:build
```

The preview command serves the site on localhost. The production build goes to
`docs/.vitepress/dist/`. The build checks local page links and writes redirects
from the previous MkDocs URLs. Links to files outside `docs/` should point
directly to their source on GitHub.

## Publish

The Documentation workflow publishes changes merged to `main` to `gh-pages`
and preserves the APT archive stored on that branch. GitHub Pages must use
**Deploy from a branch → gh-pages → / (root)**.

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
- Check published package names before updating installation commands.

Installation is organised around the package-first approach used by
[niri](https://niri-wm.github.io/niri/Getting-Started.html) and
[Hyprland](https://wiki.hypr.land/Getting-Started/Installation/).

Dated experiments belong in the [archive](archive.md), excluded from site search.
They are evidence of a specific test, not current installation instructions.
