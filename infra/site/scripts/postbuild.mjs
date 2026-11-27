// Runs after `vite build` (see the build script in package.json) and finishes the static folder in build/:
//
// 1. Copies files from docs/ that are not pages: docs/public/* to the site root (not CNAME, a GitHub Pages file)
//    and everything else, such as docs/images, to /docs/.
// 2. Writes small redirect pages for the old URLs. The VitePress site served docs pages at the root (/config,
//    /install-fedora); they now live under /docs/. Older moved pages keep pointing at their new place too.
// 3. Builds the Pagefind search index into build/pagefind/ from the docs pages (data-pagefind-body).
// 4. Redirects the roots of the old per-language references (/reference/typescript/, /reference/python/ and /reference/rust/, which
//    TypeDoc, Sphinx and rustdoc wrote) to /reference, the one API reference of the site, and writes the `_redirects` file that
//    sends their subpaths there too.
import { copyFileSync, existsSync, mkdirSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import * as pagefind from "pagefind";

const site = fileURLToPath(new URL("..", import.meta.url));
const docs = path.resolve(site, "../../docs");
const out = path.join(site, "build");
const OLD_REFERENCES = ["typescript", "python", "rust"];

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

function writeRedirect(from, target, { folder = false } = {}) {
    // A folder redirect is the folder's own index page, which the Worker serves at /from/.
    const file = folder ? path.join(out, from, "index.html") : path.join(out, `${from}.html`);
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
    install: "installation",
    configuration: "config",
    "configuration-reference": "config/configure",
    "configuration-recipes": "recipes",
    "configuration-loading": "guides/files_and_load_order",
    shortcuts: "guides/shortcuts",
    autostart: "guides/autostart",
    "window-rules": "guides/window_rules",
    "window-effects": "guides/window_effects",
    animations: "guides/animations",
    shaders: "guides/window_effects",
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
    "config/animations": "guides/animations",
    "config/shaders": "guides/window_effects",
    "config/cursors": "guides/cursors",
    "config/session_settings": "guides/session_settings",
    "config/window_menu": "guides/window_menu",
    "config/window_state_shortcuts": "guides/window_state_shortcuts",
    "config/window_snapping": "guides/window_snapping",
};
// Reference pages that were folded into other pages. Their root URLs and their /docs/ URLs both keep working.
const FOLDED = {
    "blur-fades": "background-effects",
    "bridge-examples": "bridge/examples",
    "compositor-bridge": "bridge",
    "config/runtime-api": "shell-api",
    "focus-transfer": "shell-api/focus-activation",
    "launch-feedback": "shell-api/launch-feedback",
    "shell-api/grants": "shell-api",
    "shell-dbus-api": "dbus",
    "shell-api/workspaces-monitors": "shell-api/workspaces",
    "shell-api/passphrase-prompts": "shell-api/requests",
    "shell-api/input-devices": "shell-api/input",
    "shell-api/appearance-animations": "shell-api/tweens",
    "shell-api/effects": "js-style",
    "shell-api/views": "shell-api",
    "shell-api/overrides": "shell-api",
    "shell-api/privacy": "shell-api",
    "shell-api/input-grab": "shell-api",
    "shell-api/text-insertion": "shell-api",
    "shell-api/tablet-pad-help": "shell-api",
    "events/input-shortcuts": "events",
    "events/launch-portal": "events/lifecycle",
    "events/listen-all": "events",
    "events/session": "events/lifecycle",
    "js-host-api": "shell-api/session-runtime",
    "js-idle-auth-launches": "shell-api/idle",
    "js-outputs": "shell-api/outputs",
};
for (const [from, to] of Object.entries(FOLDED)) {
    MOVED[from] = to;
    MOVED[`docs/${from}`] = to;
}
for (const [from, to] of Object.entries(MOVED)) {
    if (!pages.has(to)) throw new Error(`Redirect target docs/${to}.md does not exist (from /${from})`);
    if (writeRedirect(from, docsUrl(to))) redirects++;
}

// 3. Search index.
// Only the docs pages carry data-pagefind-body, so only they are read: docs.html and docs/**. The reference pages are not read: the index
// has one record for each class and member of the reference instead (build/reference/search-records.json, written by
// src/routes/reference/search-records.json), found by any of its three spellings. The copy button's label
// in each code block is not page text. The pages are written in British English and the site never serves another
// language, so one language is forced. Pagefind also writes its own search UI, which the site does not use (it has
// its own dialog, src/lib/components/search/); those files are removed so they are not deployed.
const { index, errors } = await pagefind.createIndex({ forceLanguage: "en-gb", excludeSelectors: [".code-head"] });
if (!index) throw new Error(`Pagefind: ${errors.join(", ")}`);
const added = await index.addDirectory({ path: out, glob: "{docs.html,docs/**/*.html}" });
if (added.errors.length) throw new Error(`Pagefind: ${added.errors.join(", ")}`);
// The reference: a record for each class and member. The `section` filter lets the dialog list them above the pages, and the title in
// each language lets it show the one that is picked.
const recordsFile = path.join(out, "reference/search-records.json");
const records = JSON.parse(readFileSync(recordsFile, "utf8"));
for (const record of records) {
    const { errors: failed } = await index.addCustomRecord({
        url: record.url,
        content: record.content,
        language: "en-gb",
        meta: {
            title: record.title.typescript,
            title_typescript: record.title.typescript,
            title_python: record.title.python,
            title_rust: record.title.rust,
            kind: record.kind,
            trail: record.trail,
        },
        filters: { section: ["reference"] },
    });
    if (failed.length) throw new Error(`Pagefind: ${record.url}: ${failed.join(", ")}`);
}
rmSync(recordsFile);
const written = await index.writeFiles({ outputPath: path.join(out, "pagefind") });
if (written.errors.length) throw new Error(`Pagefind: ${written.errors.join(", ")}`);
await pagefind.close();
for (const file of readdirSync(path.join(out, "pagefind"))) {
    if (/^pagefind-(component-ui|modular-ui|ui|highlight)\./.test(file)) rmSync(path.join(out, "pagefind", file));
}

// 4. The old per-language references, and the docs page that led to them.
if (writeRedirect("docs/reference", "/reference")) redirects++;
for (const language of OLD_REFERENCES) {
    if (writeRedirect(`reference/${language}`, "/reference", { folder: true })) redirects++;
}
writeFileSync(
    path.join(out, "_redirects"),
    `${OLD_REFERENCES.map((language) => `/reference/${language}/* /reference 301`).join("\n")}\n`,
);

console.log(
    `postbuild: copied ${copied} files from docs/, wrote ${redirects} redirects, indexed ${added.page_count} pages and ${records.length} reference entries`,
);
