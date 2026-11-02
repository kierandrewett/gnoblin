// The browser side of the search index that `yarn build` writes to /pagefind/ (scripts/postbuild.mjs). One Pagefind
// instance is shared by the page: `warm()` loads its script, its WebAssembly and its metadata, and `find()` runs a
// query against it. Result data (the page title, trail and excerpts) is fetched only for the pages that are shown.
import { parseExcerpt, stripLead, type Group, type Row } from "./hit.ts";

interface RawSub {
    title: string;
    url: string;
    excerpt: string;
}
interface RawData {
    url: string;
    excerpt: string;
    meta: { title?: string; trail?: string };
    sub_results: RawSub[];
}
interface RawResult {
    data(): Promise<RawData>;
}
interface PagefindModule {
    options(options: Record<string, unknown>): Promise<void>;
    init(): Promise<void>;
    search(query: string): Promise<{ results: RawResult[] } | null>;
}

/** Words in an excerpt. */
const EXCERPT_WORDS = 18;
/** Headings listed under each page. */
const HEADINGS_PER_PAGE = 3;
/** Wait before trying again after the index failed to load. */
const RETRY_AFTER_MS = 3000;

let loading: Promise<PagefindModule | null> | undefined;
let failedAt = 0;

/** Loads Pagefind once. Resolves to null when there is no index (for example under `yarn dev`). */
export function warm(): Promise<PagefindModule | null> {
    if (loading && (failedAt === 0 || Date.now() - failedAt < RETRY_AFTER_MS)) return loading;
    failedAt = 0;
    const attempt: Promise<PagefindModule | null> = import(/* @vite-ignore */ `${location.origin}/pagefind/pagefind.js`)
        .then(async (module: PagefindModule) => {
            await module.options({ excerptLength: EXCERPT_WORDS });
            await module.init();
            return module;
        })
        .catch(() => {
            failedAt = Date.now();
            return null;
        });
    loading = attempt;
    return attempt;
}

/** Pagefind indexes the built .html files; the site serves them without the extension. */
const clean = (url: string) => url.replace(/(\/index)?\.html(?=$|#)/, "").replace(/^$/, "/") || "/";

function toGroup(data: RawData): Group {
    const url = clean(data.url);
    const title = data.meta.title ?? url;
    const trail = data.meta.trail ?? "";
    // The first match without an anchor is in the page's own text; the others are under headings.
    const lead = data.sub_results.find((sub) => !sub.url.includes("#"));
    const headings = data.sub_results
        .filter((sub) => sub.url.includes("#") && sub.title !== title)
        .slice(0, HEADINGS_PER_PAGE);
    // With no match in the intro, the headings carry the excerpts and the page row stays short.
    const excerpt = lead?.excerpt ?? (headings.length ? "" : data.excerpt);
    const rows: Row[] = [{ kind: "page", title, trail, url, excerpt: stripLead(parseExcerpt(excerpt), title) }];
    for (const sub of headings) {
        rows.push({
            kind: "heading",
            title: sub.title,
            trail: title,
            url: clean(sub.url),
            excerpt: stripLead(parseExcerpt(sub.excerpt), sub.title),
        });
    }
    return { url, title, rows };
}

export interface Found {
    /** Pages that match. */
    total: number;
    /** Pages loaded so far. */
    loaded: number;
    /** Fetches the data of the next `count` pages. */
    more(count: number): Promise<Group[]>;
}

/** Runs a query. Resolves to null when the index is not available. */
export async function find(query: string): Promise<Found | null> {
    const module = await warm();
    if (!module) return null;
    const response = await module.search(query);
    const results = response?.results ?? [];
    const found: Found = {
        total: results.length,
        loaded: 0,
        async more(count) {
            const slice = results.slice(found.loaded, found.loaded + count);
            found.loaded += slice.length;
            return (await Promise.all(slice.map((result) => result.data()))).map(toGroup);
        },
    };
    return found;
}
