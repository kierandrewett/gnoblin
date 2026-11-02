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

To check the built pages in headless Chrome, run `yarn test:pages` after `yarn build`.

## Publish

The Infrastructure workflow builds and checks the site on pull requests, which catches broken pages.
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
