// Behaviour for every language tab group (.lang-tabs, written by src/lib/server/markdown.ts from `::: tabs`) inside an
// element: a click or an arrow key selects a language, and every group on the page follows, so a reader who picks Python
// reads Python down the page. The choice is kept for the next page. Arrow keys, Home and End move between the tabs of a
// group (WAI-ARIA tabs). The markup is complete without this script: the first language shows.
import type { Action } from "svelte/action";

const STORAGE_KEY = "gnoblin-language";

function remembered(): string | null {
    try {
        return localStorage.getItem(STORAGE_KEY);
    } catch {
        return null;
    }
}

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
        for (const panel of group.querySelectorAll<HTMLElement>('[role="tabpanel"]')) {
            panel.hidden = panel.dataset.lang !== lang;
        }
    }
}

/** The parameter is the content (for example the page's HTML): when it changes, the groups are found again. */
export const langTabs: Action<HTMLElement, unknown> = (node) => {
    const choose = (lang: string, store: boolean) => {
        select(node, lang);
        if (!store) return;
        try {
            localStorage.setItem(STORAGE_KEY, lang);
        } catch {
            // Storage is off: the choice lasts for this page.
        }
    };

    const onClick = (event: MouseEvent) => {
        if (!(event.target instanceof Element)) return;
        const tab = event.target.closest<HTMLButtonElement>('.lang-tabs [role="tab"]');
        if (tab?.dataset.lang && node.contains(tab)) choose(tab.dataset.lang, true);
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
        if (!target?.dataset.lang) return;
        event.preventDefault();
        choose(target.dataset.lang, true);
        target.focus();
    };

    const apply = () => {
        const lang = remembered();
        if (lang) select(node, lang);
    };

    node.addEventListener("click", onClick);
    node.addEventListener("keydown", onKey);
    apply();
    return {
        update: apply,
        destroy() {
            node.removeEventListener("click", onClick);
            node.removeEventListener("keydown", onKey);
        },
    };
};
