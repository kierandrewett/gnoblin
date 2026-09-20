# Maintain the docs

Edit Markdown in `docs/`. The same files appear on GitHub and the
[public site](https://kierandrewett.github.io/gnoblin/).

## Preview

From the repository root:

```sh
python3 -m venv /tmp/gnoblin-docs
/tmp/gnoblin-docs/bin/pip install -r docs-requirements.txt
/tmp/gnoblin-docs/bin/mkdocs serve
```

Open the printed localhost address. To check links and build static HTML:

```sh
/tmp/gnoblin-docs/bin/mkdocs build --strict
```

Output goes to ignored `site/`. Source links are checked and rewritten to
GitHub by `scripts/docs-links.py`.

## Publish

```sh
/tmp/gnoblin-docs/bin/mkdocs gh-deploy --strict
```

This pushes generated HTML to `gh-pages`, not your source branch.
GitHub Pages must use **Deploy from a branch → gh-pages → / (root)**.

Once the Documentation workflow is on `main`, it checks pull requests and
publishes changes automatically. See
[Material's guide](https://squidfunk.github.io/mkdocs-material/publishing-your-site/).

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
