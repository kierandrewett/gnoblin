import { mkdir, readdir, writeFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import path from "node:path";

const root = fileURLToPath(new URL("../", import.meta.url));
const docs = path.join(root, "docs");
const output = path.join(docs, ".vitepress", "dist");
const entries = await readdir(docs, { withFileTypes: true });
const base = "/gnoblin/";
const movedConfigPages = {
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
    permissions: "guides/permissions",
    "window-menu": "guides/window_menu",
    "window-state-shortcuts": "guides/window_state_shortcuts",
    "window-snapping": "guides/window_snapping",
};

async function writeRedirect(file, target) {
    await mkdir(path.dirname(file), { recursive: true });
    await writeFile(
        file,
        `<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8">
    <meta http-equiv="refresh" content="0; url=${target}">
    <link rel="canonical" href="${target}">
    <title>Gnoblin documentation moved</title>
  </head>
  <body><p>This page moved. <a href="${target}">Continue to the documentation</a>.</p></body>
</html>
`,
    );
}

for (const entry of entries) {
    if (!entry.isFile() || !entry.name.endsWith(".md") || entry.name === "index.md") continue;

    const slug = entry.name.slice(0, -3);
    const destination = path.join(output, slug);
    const target = `${base}${slug}`;
    await writeRedirect(path.join(destination, "index.html"), target);
}

for (const [oldSlug, newSlug] of Object.entries(movedConfigPages)) {
    const target = `${base}${newSlug}`;
    await writeRedirect(path.join(output, oldSlug, "index.html"), target);
    await writeRedirect(path.join(output, `${oldSlug}.html`), target);
}

const oldConfigRoutes = {
    reference: "config/configure",
    files_and_load_order: "guides/files_and_load_order",
    shortcuts: "guides/shortcuts",
    permissions: "guides/permissions",
    window_rules: "guides/window_rules",
    window_effects: "guides/window_effects",
    window_frames: "guides/window_frames",
    animations: "guides/animations",
    shaders: "guides/shaders",
    cursors: "guides/cursors",
    session_settings: "guides/session_settings",
    window_menu: "guides/window_menu",
    window_state_shortcuts: "guides/window_state_shortcuts",
    window_snapping: "guides/window_snapping",
};
for (const [oldSlug, newSlug] of Object.entries(oldConfigRoutes)) {
    const target = `${base}${newSlug}`;
    await writeRedirect(path.join(output, "config", oldSlug, "index.html"), target);
    await writeRedirect(path.join(output, "config", `${oldSlug}.html`), target);
}

await writeFile(path.join(output, ".nojekyll"), "");
