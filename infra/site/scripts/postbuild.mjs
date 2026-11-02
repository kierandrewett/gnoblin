// Runs after `vite build` (see the build script in package.json) and finishes the static folder in build/:
//
// 1. Copies files from docs/ that are not pages: docs/public/* to the site root (not CNAME, a GitHub Pages file)
//    and everything else, such as docs/images, to /docs/.
// 2. Writes small redirect pages for the old URLs. The VitePress site served docs pages at the root (/config,
//    /install-fedora); they now live under /docs/. Older moved pages keep pointing at their new place too.
// 3. Builds the Pagefind search index into build/pagefind/ from the docs pages (data-pagefind-body).
import { copyFileSync, existsSync, mkdirSync, readdirSync, writeFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import * as pagefind from "pagefind";

const site = fileURLToPath(new URL("..", import.meta.url));
const docs = path.resolve(site, "../../docs");
const out = path.join(site, "build");

function walk(dir) {
    return readdirSync(dir, { withFileTypes: true }).flatMap((entry) =>
        entry.isDirectory() ? walk(path.join(dir, entry.name)) : [path.join(dir, entry.name)],
    );
}

function copy(from, to) {
    mkdirSync(path.dirname(to), { recursive: true });
    copyFileSync(from, to);
}

// 1. Files from docs/.
let copied = 0;
for (const file of walk(docs)) {
    const rel = path.relative(docs, file);
    if (rel.endsWith(".md")) continue;
    if (rel.startsWith(`public${path.sep}`)) {
        if (rel === path.join("public", "CNAME")) continue;
        copy(file, path.join(out, path.relative("public", rel)));
    } else {
        copy(file, path.join(out, "docs", rel));
    }
    copied++;
}

// 2. Redirects from old URLs.
const pages = new Set(
    walk(docs)
        .filter((f) => f.endsWith(".md") && !f.startsWith(path.join(docs, "public")))
        .map((f) =>
            path
                .relative(docs, f)
                .split(path.sep)
                .join("/")
                .replace(/\.md$/, "")
                .replace(/(^|\/)index$/, ""),
        ),
);
const docsUrl = (slug) => (slug === "" ? "/docs" : `/docs/${slug}`);

function writeRedirect(from, target) {
    const file = path.join(out, `${from}.html`);
    // Never replace a real page (for example /bingux, which is now the Bingux page).
    if (existsSync(file) || existsSync(path.join(out, from, "index.html"))) return false;
    mkdirSync(path.dirname(file), { recursive: true });
    writeFileSync(
        file,
        `<!doctype html>
<html lang="en-GB">
<head>
<meta charset="utf-8">
<meta name="robots" content="noindex">
<meta http-equiv="refresh" content="0; url=${target}">
<link rel="canonical" href="https://gnoblin.org${target}">
<title>Moved</title>
<script>location.replace(${JSON.stringify(target)} + location.hash);</script>
</head>
<body><p>This page moved to <a href="${target}">${target}</a>.</p></body>
</html>
`,
    );
    return true;
}

let redirects = 0;
for (const slug of pages) {
    if (slug !== "" && writeRedirect(slug, docsUrl(slug))) redirects++;
}
// Pages that moved before the switch to /docs/ (from scripts/docs-vitepress-postbuild.mjs).
const MOVED = {
    configuration: "config",
    "configuration-reference": "config/configure",
    "configuration-recipes": "recipes",
    "configuration-loading": "guides/files_and_load_order",
    shortcuts: "guides/shortcuts",
    autostart: "guides/autostart",
    "window-rules": "guides/window_rules",
    "window-effects": "guides/window_effects",
    "window-frames": "guides/window_frames",
    animations: "guides/animations",
    shaders: "guides/shaders",
    cursors: "guides/cursors",
    "session-settings": "guides/session_settings",
    "window-menu": "guides/window_menu",
    "window-state-shortcuts": "guides/window_state_shortcuts",
    "window-snapping": "guides/window_snapping",
    "config/reference": "config/configure",
    "config/files_and_load_order": "guides/files_and_load_order",
    "config/shortcuts": "guides/shortcuts",
    "config/window_rules": "guides/window_rules",
    "config/window_effects": "guides/window_effects",
    "config/window_frames": "guides/window_frames",
    "config/animations": "guides/animations",
    "config/shaders": "guides/shaders",
    "config/cursors": "guides/cursors",
    "config/session_settings": "guides/session_settings",
    "config/window_menu": "guides/window_menu",
    "config/window_state_shortcuts": "guides/window_state_shortcuts",
    "config/window_snapping": "guides/window_snapping",
};
for (const [from, to] of Object.entries(MOVED)) {
    if (!pages.has(to)) throw new Error(`Redirect target docs/${to}.md does not exist (from /${from})`);
    if (writeRedirect(from, docsUrl(to))) redirects++;
}

// 3. Search index.
const { index, errors } = await pagefind.createIndex({});
if (!index) throw new Error(`Pagefind: ${errors.join(", ")}`);
const added = await index.addDirectory({ path: out });
if (added.errors.length) throw new Error(`Pagefind: ${added.errors.join(", ")}`);
const written = await index.writeFiles({ outputPath: path.join(out, "pagefind") });
if (written.errors.length) throw new Error(`Pagefind: ${written.errors.join(", ")}`);
await pagefind.close();

console.log(
    `postbuild: copied ${copied} files from docs/, wrote ${redirects} redirects, indexed ${added.page_count} pages`,
);
