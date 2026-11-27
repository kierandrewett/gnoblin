// Behaviour for every language tab group (.lang-tabs, written by src/lib/server/markdown.ts from `::: tabs`) inside an
// element: a click or an arrow key picks a language for the whole site (setLanguage in state.svelte.ts), and every
// group on the page follows, so a reader who picks Python reads Python down the page. app.html applies the stored language (or `?lang=`) to <html data-lang> before the first
// paint and src/lib/styles/content.css shows it from that attribute, so the markup needs no script to show the right
// panel; this action keeps the tabs' ARIA state in step. Arrow keys, Home and End move between
// the tabs of a group (WAI-ARIA tabs).
import type { Action } from "svelte/action";
import { LANGUAGE_EVENT, LANGUAGES, setLanguage, type LanguageId } from "../state.svelte.ts";

const isLanguage = (value: unknown): value is LanguageId => LANGUAGES.some((l) => l.id === value);

/** Shows `lang` in every group that has it; a group without it keeps what it shows. */
function select(root: HTMLElement, lang: string) {
    for (const group of root.querySelectorAll<HTMLElement>(".lang-tabs")) {
        const tabs = [...group.querySelectorAll<HTMLButtonElement>('[role="tab"]')];
        if (!tabs.some((tab) => tab.dataset.lang === lang)) continue;
        for (const tab of tabs) {
            const on = tab.dataset.lang === lang;
            tab.setAttribute("aria-selected", String(on));
            tab.tabIndex = on ? 0 : -1;
        }
        // Inactive panels are hidden by CSS from <html data-lang>, which also keeps them out of the accessibility tree.
    }
}

/** The parameter is the content (for example the page's HTML): when it changes, the groups are found again. */
export const langTabs: Action<HTMLElement, unknown> = (node) => {
    const onClick = (event: MouseEvent) => {
        if (!(event.target instanceof Element)) return;
        const tab = event.target.closest<HTMLButtonElement>('.lang-tabs [role="tab"]');
        if (isLanguage(tab?.dataset.lang) && node.contains(tab)) setLanguage(tab.dataset.lang);
    };

    const onKey = (event: KeyboardEvent) => {
        if (!(event.target instanceof Element)) return;
        const tab = event.target.closest<HTMLButtonElement>('.lang-tabs [role="tab"]');
        const list = tab?.closest('[role="tablist"]');
        if (!tab || !list) return;
        const tabs = [...list.querySelectorAll<HTMLButtonElement>('[role="tab"]')];
        const at = tabs.indexOf(tab);
        const last = tabs.length - 1;
        const next = { ArrowRight: at === last ? 0 : at + 1, ArrowLeft: at === 0 ? last : at - 1, Home: 0, End: last }[
            event.key
        ];
        const target = next === undefined ? undefined : tabs[next];
        if (!isLanguage(target?.dataset.lang)) return;
        event.preventDefault();
        setLanguage(target.dataset.lang);
        target.focus();
    };

    const onLanguage = (event: Event) => select(node, (event as CustomEvent<string>).detail);

    const apply = () => {
        const lang = document.documentElement.dataset.lang;
        if (lang) select(node, lang);
    };

    node.addEventListener("click", onClick);
    node.addEventListener("keydown", onKey);
    window.addEventListener(LANGUAGE_EVENT, onLanguage);
    apply();
    return {
        update: apply,
        destroy() {
            node.removeEventListener("click", onClick);
            node.removeEventListener("keydown", onKey);
            window.removeEventListener(LANGUAGE_EVENT, onLanguage);
        },
    };
};
