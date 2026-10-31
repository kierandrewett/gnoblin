# Documentation site

Edit the Markdown in `docs/`. VitePress builds the
[public site](https://gnoblin.org/) from those files.

## Preview

From the `infra/website/` directory. It holds the VitePress project and reads the pages from `docs/`:

```sh
npm ci
npm run docs:dev
```

Open the printed localhost address. To check links and build static HTML:

```sh
npm run docs:build
```

The preview command serves the site on localhost. The production build goes to
`infra/website/.vitepress/dist/`. The build checks local page links and writes redirects
from the previous MkDocs URLs. Links to files outside `docs/` should point
directly to their source on GitHub.

## Publish

The Infrastructure workflow builds the site on pull requests, which catches broken pages.
On `main` it plans the deployment. It uploads the build only in these cases:

- The repository variable `INFRA_AUTO_APPLY` is `true`.
- You run the workflow by hand with `apply` set.

Terraform uploads the build to the `gnoblin-website` Worker. See `infra/README.md` in the source repository.

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
- Capture screenshots in a fresh devkit profile or VM and inspect them for personal data before publishing.
- Show real interface activity, without placeholder output. Put commands and explanatory text in prose or code blocks, not in staged terminal screenshots.
- Check published package names before updating installation commands.

Installation is organised around the package-first approach used by
[niri](https://niri-wm.github.io/niri/Getting-Started.html) and
[Hyprland](https://wiki.hypr.land/Getting-Started/Installation/).

Dated experiments and superseded design proposals are kept under
`design/archive/` in the source repository. They are not part of the published
user documentation.
