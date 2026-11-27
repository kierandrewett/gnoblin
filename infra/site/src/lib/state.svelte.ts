// Client state shared across the page: the search dialog and the polite live region for copy confirmations.

export const search = $state({ open: false });

export const announcer = $state({ message: "" });

/** The languages of the SDKs, in the order of every `::: tabs` group. */
export const LANGUAGES = [
    { id: "typescript", label: "TypeScript" },
    { id: "python", label: "Python" },
    { id: "rust", label: "Rust" },
] as const;
export type LanguageId = (typeof LANGUAGES)[number]["id"];

export const LANGUAGE_KEY = "gnoblin-language";
/** Fired on `window` when the reader picks a language; the tab groups of the page follow. */
export const LANGUAGE_EVENT = "gnoblin-language";

/** Picks a language for the whole site: every tab group follows, and the choice is kept for the next page. */
export function setLanguage(lang: LanguageId) {
    document.documentElement.dataset.lang = lang;
    try {
        localStorage.setItem(LANGUAGE_KEY, lang);
    } catch {
        // Storage is off: the choice lasts until the page is closed.
    }
    window.dispatchEvent(new CustomEvent(LANGUAGE_EVENT, { detail: lang }));
}

/** Copies text and announces it in the live region (the layout renders `announcer.message`). */
export async function copyText(text: string): Promise<boolean> {
    try {
        await navigator.clipboard.writeText(text);
    } catch {
        // Clipboard API unavailable (for example over plain HTTP): fall back to a selected textarea.
        const area = document.createElement("textarea");
        area.value = text;
        area.setAttribute("readonly", "");
        area.style.position = "fixed";
        area.style.opacity = "0";
        document.body.append(area);
        area.select();
        const ok = document.execCommand("copy");
        area.remove();
        if (!ok) return false;
    }
    announcer.message = "";
    queueMicrotask(() => (announcer.message = "Copied to the clipboard"));
    return true;
}
