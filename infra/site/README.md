# gnoblin.org website

The Gnoblin website and documentation, built with SvelteKit and prerendered into a plain static folder. The design
comes from the Gnoblin design source (`SPEC.md` and the Doop canvas "Gnoblin website").

## Run it

Use Node 22.17 or later and Yarn through Corepack:

```sh
corepack enable
cd infra/site
yarn install --immutable
yarn dev        # development server with live reload; search needs a build
yarn build      # writes the static site to build/
yarn preview    # serves build/ like the Cloudflare Worker does, at http://localhost:4173
```

`yarn build` runs `vite build`, then `scripts/postbuild.mjs`. The postbuild step copies files from `docs/` that are
not pages, writes redirect pages for old URLs and builds the Pagefind search index into `build/pagefind/`.

## Checks

```sh
yarn check        # svelte-check and TypeScript
yarn lint         # ESLint and Prettier
yarn test:pages   # after yarn build: headless Chrome over key pages
```

`yarn test:pages` opens the key pages in light and dark, at desktop, tablet and phone sizes. It fails on console
errors, failed requests, horizontal overflow and images that do not load, and checks that search finds a page. It
uses `google-chrome`, `google-chrome-stable` or `chromium` from your `PATH`; set `CHROME` to use another browser.
Add `--shots DIR` to save a screenshot of every page.

The build itself fails when:

- a link in `docs/` points to a page or file that does not exist;
- a link points to a heading that does not exist, unless it is listed in `KNOWN_MISSING_ANCHORS` in
  `vite.config.ts`. Each entry there is a bug in `docs/`; fix the link and remove the entry;
- a page in `docs/` is in neither the sidebar nor the `UNLISTED` list in `src/lib/nav.ts`, or the sidebar names a
  page that does not exist.

## How the docs are built

The pages are read at build time from the repository's `docs/` folder. Nothing is copied into this folder.

- `src/lib/server/docs.ts` finds the pages, resolves links and runs the sidebar check.
- `src/lib/server/markdown.ts` turns Markdown into HTML. It supports what the pages use: GitHub-flavoured Markdown,
  `::: details Title` and the callout containers (`::: tip`, `::: warning`, `::: danger`, `::: info`), `{#id}` after
  a heading and fenced code highlighted by Shiki.
- `src/routes/docs/[...slug]/` renders each page at `/docs/<path>`: `docs/install-fedora.md` is
  `/docs/install-fedora` and `docs/recipes/index.md` is `/docs/recipes`.
- Files in `docs/public/` are served at the site root and `docs/images/` at `/docs/images/`.
- The old site served pages at the root, such as `/install-fedora`. Those URLs redirect to `/docs/...`.

Links between pages work as written in `docs/`: `install-fedora.md`, `../guides/shortcuts.md#anchor` or
`/config/configure/input`.

### Add a page

1. Write the page in `docs/`, starting with a `# Title` line. The paragraph after the title is shown as the lede.
2. Add it to the sidebar in `src/lib/nav.ts`, in the section and order where readers should find it. A page that
   should not be in the sidebar goes in `UNLISTED` instead.
3. Run `yarn build`.

## Where things are

| Path                      | What it holds                                                                        |
| ------------------------- | ------------------------------------------------------------------------------------ |
| `src/lib/styles/`         | Design tokens (`tokens.css`), base styles, content components and docs prose         |
| `src/lib/components/`     | Layout, content, install, search, Bingux and small UI components                     |
| `src/lib/bingux/`         | The Bingux design system for the `/bingux` page, scoped to `data-theme="bingux"`     |
| `src/lib/brand/`          | The logo slot (see below)                                                            |
| `src/lib/site.ts`         | Project links, the setup command and the systems with an install guide               |
| `static/icons.svg`        | Icon sprite: Bingux icons (MIT, `static/icons.COPYING`) and Simple Icons marks (CC0) |
| `static/shots/`           | Screenshots as WebP and PNG at 1x and 2x                                             |
| `scripts/build-icons.mjs` | Rebuilds the icon sprite and copies the Bingux app icons: `yarn icons`               |
| `scripts/build-shots.mjs` | Rebuilds the screenshots from the original captures: `yarn shots`                    |
| `scripts/check-pages.mjs` | The headless Chrome check                                                            |

The icon sprite and the screenshots are committed, so a build needs neither the Bingux sources nor the original
captures. Run `yarn icons` or `yarn shots` only to change them; set `BINGUX_MAIN` or `SHOTS_DIR` if those sources are
not in the default place.

Appearance: the site follows the system's light or dark setting until the reader chooses one with the button in the
header. The choice is stored in `localStorage` and applied in `src/app.html` before the page is drawn.

## Logo

The logo has not been chosen yet. `src/lib/brand/` holds placeholders:

- `mark.svg`: a rounded square with a lowercase g, drawn in `currentColor`, used in the header and on the home page;
- `logo.svg`: the word "gnoblin" as text, for use outside the pages (the header sets the name as text);
- `favicon.svg`: the mark in the accent colour, as the favicon.

To use a logo concept, replace these three files, for example with `gnoblin-mark.svg`, `gnoblin-logo.svg` and
`gnoblin-favicon.svg` from `~/dev/gnoblin-icon/dist`. Nothing else needs to change: the mark keeps its own aspect
ratio and takes the colour of the text around it.
