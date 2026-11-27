// Small helpers for the HTML the reference pages hold: every spelled thing is written once for each language that spells it differently,
// and styles/reference.css shows the variant of the language the reader picked (`data-only` on the element, a list of language ids).
import { LANGUAGE_IDS, type LanguageId, type Spelled } from "./types.ts";

export function esc(text: string): string {
    return text.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");
}

/** HTML for each language, written as one string when the languages agree and as `data-only` variants where they differ. */
export function spell(html: Record<LanguageId, string>): string {
    const groups: { langs: LanguageId[]; html: string }[] = [];
    for (const lang of LANGUAGE_IDS) {
        const group = groups.find((g) => g.html === html[lang]);
        if (group) group.langs.push(lang);
        else groups.push({ langs: [lang], html: html[lang] });
    }
    if (groups.length === 1) return groups[0]!.html;
    return groups.map((g) => `<span data-only="${g.langs.join(" ")}">${g.html}</span>`).join("");
}

/** Escaped text for each language: a name, a keyword, a type. */
export function spelled(value: Spelled): string {
    return spell({ typescript: esc(value.typescript), python: esc(value.python), rust: esc(value.rust) });
}

/** The same for a value that is not spelled: HTML written for every language alike. */
export function same(html: string): Record<LanguageId, string> {
    return { typescript: html, python: html, rust: html };
}

/** The `data-only` attribute for a piece of markup that belongs to some languages only. */
export function only(...langs: LanguageId[]): string {
    return `data-only="${langs.join(" ")}"`;
}
