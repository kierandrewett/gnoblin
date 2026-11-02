import { execFileSync } from "node:child_process";
import { createReadStream, existsSync, statSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import adapter from "@sveltejs/adapter-static";
import { sveltekit } from "@sveltejs/kit/vite";
import { defineConfig, type Plugin } from "vite";

const here = path.dirname(fileURLToPath(import.meta.url));
// The pages are read from the repository's docs folder at build time; nothing is copied into this folder.
const docsDir = path.resolve(here, "../../docs");

// The commit and time of this build, shown in the footer. CI sets GITHUB_SHA; a local build asks Git.
const buildSha =
    process.env.GITHUB_SHA ?? execFileSync("git", ["rev-parse", "HEAD"], { cwd: here, encoding: "utf8" }).trim();
const builtAt = new Date().toISOString().replace(/\.\d+Z$/, "Z");

const types: Record<string, string> = {
    ".png": "image/png",
    ".jpg": "image/jpeg",
    ".webp": "image/webp",
    ".svg": "image/svg+xml",
    ".json": "application/json",
    ".lua": "text/plain; charset=utf-8",
};

/** Development only: serve docs/images (and other non-page files) at /docs/... and docs/public at the site root,
 *  the same URLs scripts/postbuild.mjs copies them to in the static build. */
function docsAssets(): Plugin {
    return {
        name: "gnoblin-docs-assets",
        apply: "serve",
        configureServer(server) {
            server.middlewares.use((req, res, next) => {
                const url = decodeURIComponent((req.url ?? "/").split("?")[0] ?? "/");
                const candidates = url.startsWith("/docs/")
                    ? [path.join(docsDir, url.slice("/docs/".length))]
                    : [path.join(docsDir, "public", url)];
                for (const file of candidates) {
                    if (!file.startsWith(docsDir) || file.endsWith(".md")) continue;
                    if (existsSync(file) && statSync(file).isFile()) {
                        res.setHeader("Content-Type", types[path.extname(file)] ?? "application/octet-stream");
                        createReadStream(file).pipe(res);
                        return;
                    }
                }
                next();
            });
        },
    };
}

/** Links in docs/ to headings that do not exist (the old site did not check anchors). Each one is a bug to fix in
 *  docs/; it is listed here so the build passes until then, and the list should only shrink. Any other missing
 *  anchor fails the build. */
const KNOWN_MISSING_ANCHORS: Record<string, true> = {
    "/docs/bridge#api-127-shell-presentation-requests": true,
    "/docs/bridge#api-130-wm-menu-actions": true,
    "/docs/bridge#api-version-121-session-locking": true,
    "/docs/bridge#api-version-175-shell-ui-sessions": true,
    "/docs/bridge#track-lock-state": true,
    "/docs/bridge#windows-and-controls": true,
    "/docs/bridge/examples#example-watch-the-window-list": true,
    "/docs/bridge/input-and-focus#api-version-132-xdg-activation-focus-and-session-logout": true,
    "/docs/bridge/shortcuts-and-events#api-version-110-shortcut-focus-grants": true,
    "/docs/bridge/shortcuts-and-events#api-version-111-dynamic-shortcuts": true,
    "/docs/bridge/shortcuts-and-events#api-version-12-layer-surfaces": true,
    "/docs/cli#animations": true,
    "/docs/cli#workspaces-and-monitors": true,
    "/docs/compositor-bridge#api-version-132-xdg-activation-focus-and-session-logout": true,
    "/docs/config#api-reference": true,
    "/docs/config/configure#window-management": true,
    "/docs/config/runtime-api#immediate-window-snapshots": true,
    "/docs/config/runtime-api#shortcut-state-and-capture": true,
    "/docs/guides/animations#per-surface-animations": true,
    "/docs/guides/session_settings#native-features": true,
    "/docs/shell-api#monitor-privacy-screens": true,
    "/docs/shell-api#shortcut-state-and-capture": true,
    "/docs/shell-api#tablet-pad-help": true,
    "/docs/shell-api/keyboard-shortcuts#shortcut-state-and-capture": true,
};

export default defineConfig({
    define: {
        __BUILD_SHA__: JSON.stringify(buildSha),
        __BUILT_AT__: JSON.stringify(builtAt),
        __DOCS_DIR__: JSON.stringify(docsDir),
    },
    server: { fs: { allow: [docsDir] } },
    plugins: [
        docsAssets(),
        sveltekit({
            compilerOptions: { runes: true },
            adapter: adapter({ pages: "build", assets: "build", strict: true }),
            prerender: {
                entries: ["*", "/404"],
                // A broken link or anchor fails the build. Files from docs/ (images, docs/public) are copied into
                // the output by scripts/postbuild.mjs, so the crawler cannot fetch them yet: check they exist instead.
                handleHttpError: ({ path: url, referrer, message }) => {
                    const rel = decodeURIComponent(url);
                    const file = rel.startsWith("/docs/")
                        ? path.join(docsDir, rel.slice("/docs/".length))
                        : path.join(docsDir, "public", rel);
                    if (!file.endsWith(".md") && existsSync(file) && statSync(file).isFile()) return;
                    throw new Error(`${message} (linked from ${referrer ?? "an entry"})`);
                },
                handleMissingId: ({ path: url, id, message }) => {
                    if (KNOWN_MISSING_ANCHORS[`${url}#${id}`]) return;
                    throw new Error(message);
                },
            },
        }),
    ],
});
