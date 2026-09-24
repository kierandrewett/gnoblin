import { mkdir, writeFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import path from "node:path";

const root = fileURLToPath(new URL("../", import.meta.url));
const docs = path.join(root, "docs");
const output = path.join(docs, ".vitepress", "dist");
const base = "/gnoblin/";
const movedPages = {
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
const movedConfigPages = {
    reference: "config/configure",
    files_and_load_order: "guides/files_and_load_order",
    shortcuts: "guides/shortcuts",
    window_rules: "guides/window_rules",
    window_effects: "guides/window_effects",
    window_frames: "guides/window_frames",
    animations: "guides/animations",
    shaders: "guides/shaders",
    cursors: "guides/cursors",
    permissions: "guides/permissions",
    session_settings: "guides/session_settings",
    window_menu: "guides/window_menu",
    window_state_shortcuts: "guides/window_state_shortcuts",
    window_snapping: "guides/window_snapping",
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

for (const [oldSlug, newSlug] of Object.entries(movedPages)) {
    const target = `${base}${newSlug}`;
    await writeRedirect(path.join(output, oldSlug, "index.html"), target);
    await writeRedirect(path.join(output, `${oldSlug}.html`), target);
}

for (const [oldSlug, newSlug] of Object.entries(movedConfigPages)) {
    const target = `${base}${newSlug}`;
    if (oldSlug !== "autostart") {
        await writeRedirect(path.join(output, "config", oldSlug, "index.html"), target);
        await writeRedirect(path.join(output, `config/${oldSlug}.html`), target);
    }
}

await writeFile(path.join(output, ".nojekyll"), "");
