// How the API reference sits in the search. The index has one record for each class and member of the reference (scripts/postbuild.mjs adds
// them, from src/routes/reference/search-records.json), found by any of its three spellings and tagged with the `section` filter. The dialog
// lists them above the pages, with the name in the language the reader picked. Everything that knows about this lives here and in the few
// lines that call it, so the one search box of the reference design can replace the dialog without touching the index.
import { LANGUAGES, type LanguageId } from "#lib/state.svelte.ts";
import type { Row } from "./hit.ts";

/** The value of the `section` filter of a reference record. A docs page has none. */
const SECTION = "reference";

/** Only the reference's records. */
export const REFERENCE_ONLY = { section: SECTION };
/** Everything but the reference's records: the docs pages. */
export const WITHOUT_REFERENCE = { section: { not: SECTION } };

/** Reference records listed first, so a reader looking for `focus` sees the member before the guide. */
export const REFERENCE_LIMIT = 8;

export const SECTION_LABEL = { reference: "API reference", pages: "Pages" } as const;

export interface RecordData {
    url: string;
    meta: Record<string, string | undefined>;
}

/** The language of the page: what <html data-lang> says, TypeScript when it says nothing. */
export function pickedLanguage(): LanguageId {
    const lang = document.documentElement.dataset.lang;
    return LANGUAGES.find((l) => l.id === lang)?.id ?? "typescript";
}

export function languageLabel(lang: LanguageId): string {
    return LANGUAGES.find((l) => l.id === lang)!.label;
}

export function isReferenceRecord(data: RecordData): boolean {
    return data.meta.kind !== undefined && data.meta.trail !== undefined;
}

/** The row of a reference record: its name in the picked language, what it is and where it lives. */
export function referenceRow(data: RecordData, url: string): Row {
    const lang = pickedLanguage();
    return {
        kind: "api",
        title: data.meta[`title_${lang}`] ?? data.meta.title ?? url,
        trail: data.meta.trail ?? "",
        label: data.meta.kind ?? "",
        url,
        excerpt: [],
    };
}
