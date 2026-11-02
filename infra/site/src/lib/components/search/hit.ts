/** A piece of text, marked where it matched the query. */
export interface Part {
    text: string;
    mark: boolean;
}

/** One link in the results: a page, or a heading inside a page. */
export interface Row {
    kind: "page" | "heading";
    title: string;
    /** Section path of the page, such as "Get started › Install". */
    trail: string;
    url: string;
    /** The text around the match. Empty when the page has nothing to show. */
    excerpt: Part[];
}

/** A matching page and the headings in it that match. The first row is the page. */
export interface Group {
    url: string;
    title: string;
    rows: Row[];
}

/** Suggestions for the empty state. Each one finds pages in the built index (checked by `yarn test:pages`). */
export const SUGGESTIONS = ["install fedora", "keyboard shortcuts", "window rules", "mouse", "animations"];

/** The words of the query, trimmed to a stem so that "windows" also marks "Window". */
function stems(query: string): string[] {
    return query
        .trim()
        .split(/\s+/)
        .filter(Boolean)
        .map((word) => {
            const stem = word.replace(/(ing|ed|es|s)$/i, "");
            return (stem.length >= 3 ? stem : word).replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
        });
}

/** Splits `text` where a word of the query starts, for titles and trails. */
export function highlight(text: string, query: string): Part[] {
    const words = stems(query);
    if (!words.length) return [{ text, mark: false }];
    const re = new RegExp(`(?<![\\p{L}\\p{N}])(${words.join("|")})`, "giu");
    // split() with a capturing group puts the matches at the odd indexes.
    return text
        .split(re)
        .map((part, i) => ({ text: part, mark: i % 2 === 1 }))
        .filter((part) => part.text);
}

/**
 * Turns a Pagefind excerpt into parts. Pagefind wraps matches in <mark> and writes < and > as entities, and nothing
 * else is HTML, so the parts are rendered as text and never as markup.
 */
export function parseExcerpt(html: string): Part[] {
    const parts: Part[] = [];
    let mark = false;
    for (const piece of html.split(/(<\/?mark>)/)) {
        if (piece === "<mark>") mark = true;
        else if (piece === "</mark>") mark = false;
        else if (piece) parts.push({ text: piece.replaceAll("&lt;", "<").replaceAll("&gt;", ">"), mark });
    }
    return parts;
}

/** Pagefind starts an excerpt with the heading it sits under ("Fedora. Install Gnoblin ..."). Drops that repeat. */
export function stripLead(parts: Part[], title: string): Part[] {
    const lead = `${title}.`;
    const text = parts.map((part) => part.text).join("");
    if (!text.startsWith(lead)) return parts;
    let drop = lead.length + (text[lead.length] === " " ? 1 : 0);
    const kept: Part[] = [];
    for (const part of parts) {
        if (drop >= part.text.length) drop -= part.text.length;
        else {
            kept.push({ ...part, text: part.text.slice(drop) });
            drop = 0;
        }
    }
    return kept;
}
