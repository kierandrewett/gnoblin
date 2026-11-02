// The documentation pages, read at build time from the repository's docs/ folder (never copied into this app).
import { existsSync, readdirSync, readFileSync, statSync } from "node:fs";
import path from "node:path";
import type { DocsPage } from "../docs.ts";
import { docsHref, NAV_ORDER, UNLISTED, type NavEntry } from "../nav.ts";
import { renderMarkdown } from "./markdown.ts";

export const DOCS_DIR = __DOCS_DIR__;

function walk(dir: string): string[] {
    return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
        const full = path.join(dir, entry.name);
        if (entry.isDirectory()) return entry.name === "public" ? [] : walk(full);
        return entry.name.endsWith(".md") ? [path.relative(DOCS_DIR, full).split(path.sep).join("/")] : [];
    });
}

let pages: Map<string, string> | undefined;
/** slug -> file under docs/ ("" -> index.md). */
export function docsPages(): Map<string, string> {
    if (pages) return pages;
    // index.md is the folder's page: "index.md" -> "", "recipes/index.md" -> "recipes".
    const slugOf = (file: string) => file.replace(/\.md$/, "").replace(/(^|\/)index$/, "");
    pages = new Map(walk(DOCS_DIR).map((file) => [slugOf(file), file]));
    checkSidebar(pages);
    return pages;
}

/** Every page must be in the sidebar (src/lib/nav.ts) or in UNLISTED, and every sidebar entry must exist. */
function checkSidebar(all: Map<string, string>) {
    const listed = new Set([...NAV_ORDER.map((e) => e.slug), ...UNLISTED]);
    const problems: string[] = [];
    for (const [slug, file] of all) {
        if (!listed.has(slug)) problems.push(`docs/${file} is not in the sidebar (src/lib/nav.ts NAV) or UNLISTED`);
    }
    for (const slug of listed) {
        if (!all.has(slug)) problems.push(`src/lib/nav.ts lists "${slug}", but docs/ has no such page`);
    }
    const seen = new Set<string>();
    for (const { slug } of NAV_ORDER) {
        if (seen.has(slug)) problems.push(`src/lib/nav.ts lists "${slug}" twice`);
        seen.add(slug);
    }
    if (problems.length) throw new Error(`Documentation sidebar check failed:\n${problems.join("\n")}`);
}

/** Resolves a link or image path written in docs/<fromFile> to its URL on this site, or null when nothing is there. */
export function resolveDocsLink(target: string, fromFile: string): string | null {
    const hashAt = target.indexOf("#");
    const raw = decodeURI(hashAt === -1 ? target : target.slice(0, hashAt));
    const hash = hashAt === -1 ? "" : target.slice(hashAt);
    const rel = raw.startsWith("/")
        ? path.posix.normalize(raw.slice(1))
        : path.posix.normalize(path.posix.join(path.posix.dirname(fromFile), raw));
    if (rel.startsWith("..")) return null;

    // Like VitePress: drop .md or .html, then a trailing slash ("../bridge/.md" is the bridge/ index page).
    const slug = rel
        .replace(/\.(md|html)$/, "")
        .replace(/\/$/, "")
        .replace(/(^|\/)index$/, "")
        .replace(/^\.$/, "");
    if (docsPages().has(slug)) return docsHref(slug) + hash;

    // Files other than pages: docs/public is served at the site root, anything else under /docs/.
    const clean = rel.replace(/\/$/, "");
    const isFile = (p: string) => existsSync(p) && statSync(p).isFile();
    if (!clean.endsWith(".md") && isFile(path.join(DOCS_DIR, "public", clean))) return `/${clean}${hash}`;
    if (!clean.endsWith(".md") && !clean.startsWith("public/") && isFile(path.join(DOCS_DIR, clean))) {
        return `/docs/${clean}${hash}`;
    }
    return null;
}

const rendered = new Map<string, Promise<DocsPage>>();

export function loadDocsPage(slug: string): Promise<DocsPage> | null {
    const file = docsPages().get(slug);
    if (file === undefined) return null;
    let page = rendered.get(slug);
    if (!page) {
        page = (async () => {
            const source = readFileSync(path.join(DOCS_DIR, file), "utf8");
            const md = await renderMarkdown(source, { file, docsDir: DOCS_DIR, resolveLink: resolveDocsLink });
            const at = NAV_ORDER.findIndex((e) => e.slug === slug);
            const entry = NAV_ORDER[at];
            const link = (e: NavEntry | undefined) => (e ? { label: e.label, href: docsHref(e.slug) } : null);
            return {
                slug,
                file,
                ...md,
                trail: entry ? entry.trail : [],
                section: entry ? entry.section.id : null,
                prev: at > 0 ? link(NAV_ORDER[at - 1]) : null,
                next: at >= 0 ? link(NAV_ORDER[at + 1]) : null,
            };
        })();
        rendered.set(slug, page);
    }
    return page;
}
