<script lang="ts">
    // Search the docs: Ctrl+K, Cmd+K or "/" opens it, Esc closes it and returns focus. Arrow keys move through the
    // results, Enter opens one. Results come from the Pagefind index that `yarn build` writes to /pagefind/.
    import { goto } from "$app/navigation";
    import { search } from "#lib/state.svelte.ts";
    import Icon from "../ui/Icon.svelte";
    import type { Hit } from "./hit.ts";
    import SearchResult from "./SearchResult.svelte";

    interface PagefindSub {
        title: string;
        url: string;
        excerpt: string;
    }
    interface PagefindData {
        url: string;
        meta: { title?: string; trail?: string };
        sub_results: PagefindSub[];
    }
    interface Pagefind {
        options(opts: Record<string, unknown>): Promise<void>;
        debouncedSearch(
            query: string,
            opts?: unknown,
            ms?: number,
        ): Promise<{ results: { data(): Promise<PagefindData> }[] } | null>;
    }

    const PAGES = 5;
    const SECTIONS = 5;

    let dialog: HTMLDialogElement;
    let input = $state<HTMLInputElement>();
    let query = $state("");
    let pages = $state<Hit[]>([]);
    let sections = $state<Hit[]>([]);
    let selected = $state(0);
    let status = $state<"idle" | "searching" | "done" | "unavailable">("idle");
    const hits = $derived([...pages, ...sections]);

    let pagefind: Promise<Pagefind | null> | undefined;
    function loadPagefind(): Promise<Pagefind | null> {
        pagefind ??= import(/* @vite-ignore */ `${location.origin}/pagefind/pagefind.js`)
            .then(async (module: Pagefind) => {
                await module.options({ excerptLength: 12 });
                return module;
            })
            .catch(() => null);
        return pagefind;
    }

    // Pagefind indexes the built .html files; the site serves them without the extension.
    const clean = (url: string) => url.replace(/(\/index)?\.html(?=$|#)/, "").replace(/^$/, "/") || "/";

    $effect(() => {
        if (search.open && !dialog.open) {
            dialog.showModal();
            input?.select();
            void loadPagefind();
        } else if (!search.open && dialog.open) {
            dialog.close();
        }
    });

    let token = 0;
    async function run(q: string) {
        const mine = ++token;
        if (!q.trim()) {
            pages = [];
            sections = [];
            status = "idle";
            return;
        }
        status = "searching";
        const api = await loadPagefind();
        if (!api) {
            status = "unavailable";
            return;
        }
        const res = await api.debouncedSearch(q, {}, 120);
        if (res === null || mine !== token) return;
        const data = await Promise.all(res.results.slice(0, 12).map((r) => r.data()));
        if (mine !== token) return;
        pages = data.slice(0, PAGES).map((d) => ({
            kind: "page",
            title: d.meta.title ?? clean(d.url),
            path: d.meta.trail ?? "",
            url: clean(d.url),
        }));
        sections = data
            .flatMap((d) =>
                d.sub_results
                    .filter((s) => s.url.includes("#") && s.title !== d.meta.title)
                    .slice(0, 2)
                    .map((s) => ({
                        kind: "section" as const,
                        title: s.title,
                        path: d.meta.title ?? "",
                        url: clean(s.url),
                    })),
            )
            .slice(0, SECTIONS);
        selected = 0;
        status = "done";
    }

    function open(hit: Hit | undefined) {
        if (!hit) return;
        search.open = false;
        void goto(hit.url);
    }

    function onKey(event: KeyboardEvent) {
        if (event.key === "ArrowDown" || event.key === "ArrowUp") {
            event.preventDefault();
            if (!hits.length) return;
            const step = event.key === "ArrowDown" ? 1 : -1;
            selected = (selected + step + hits.length) % hits.length;
            document.getElementById(`search-hit-${selected}`)?.scrollIntoView({ block: "nearest" });
        } else if (event.key === "Enter") {
            event.preventDefault();
            open(hits[selected]);
        }
    }

    function onGlobalKey(event: KeyboardEvent) {
        const typing =
            event.target instanceof HTMLElement &&
            (event.target.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(event.target.tagName));
        if ((event.key === "k" || event.key === "K") && (event.ctrlKey || event.metaKey)) {
            event.preventDefault();
            search.open = !search.open;
        } else if (event.key === "/" && !typing && !search.open) {
            event.preventDefault();
            search.open = true;
        }
    }
</script>

<svelte:window onkeydown={onGlobalKey} />

<!-- A click on the backdrop closes the dialog; Esc is native. -->
<dialog
    bind:this={dialog}
    class="search"
    aria-label="Search the docs"
    onclose={() => (search.open = false)}
    onclick={(event) => {
        if (event.target === dialog) search.open = false;
    }}
>
    <div class="panel">
        <div class="input">
            <Icon name="search" size={22} />
            <input
                bind:this={input}
                bind:value={query}
                oninput={() => run(query)}
                onkeydown={onKey}
                type="search"
                placeholder="Search the docs"
                aria-label="Search the docs"
                role="combobox"
                aria-expanded={hits.length > 0}
                aria-controls="search-results"
                aria-activedescendant={hits.length ? `search-hit-${selected}` : undefined}
                aria-autocomplete="list"
                autocomplete="off"
                spellcheck="false"
            />
            <kbd class="kbd esc">Esc</kbd>
            <button type="button" class="cancel" onclick={() => (search.open = false)}>Cancel</button>
        </div>
        <div class="results" id="search-results" role="listbox" aria-label="Results">
            {#if pages.length}
                <div role="group" aria-labelledby="search-pages">
                    <h2 id="search-pages">Pages</h2>
                    {#each pages as hit, i (hit.url)}
                        <SearchResult
                            {hit}
                            {query}
                            id="search-hit-{i}"
                            selected={i === selected}
                            onselect={() => open(hit)}
                            onhover={() => (selected = i)}
                        />
                    {/each}
                </div>
            {/if}
            {#if sections.length}
                <div role="group" aria-labelledby="search-sections">
                    <h2 id="search-sections">Sections</h2>
                    {#each sections as hit, i (hit.url)}
                        <SearchResult
                            {hit}
                            {query}
                            id="search-hit-{pages.length + i}"
                            selected={pages.length + i === selected}
                            onselect={() => open(hit)}
                            onhover={() => (selected = pages.length + i)}
                        />
                    {/each}
                </div>
            {/if}
        </div>
        <p
            class={["status", (status === "unavailable" || (status === "done" && !hits.length)) && "shown"]}
            aria-live="polite"
        >
            {#if status === "unavailable"}
                Search works in the built site. Run yarn build, then yarn preview.
            {:else if status === "done" && !hits.length}
                No results for "{query}".
            {:else if status === "done"}
                <span class="visually-hidden">{hits.length} results</span>
            {/if}
        </p>
        <div class="foot" aria-hidden="true">
            <span
                ><kbd class="kbd"><Icon name="arrow-up" size={12} /></kbd><kbd class="kbd"
                    ><Icon name="arrow-down" size={12} /></kbd
                > Move</span
            >
            <span><kbd class="kbd">Enter</kbd> Open</span>
            <span><kbd class="kbd">Esc</kbd> Close</span>
        </div>
    </div>
</dialog>

<style>
    .search {
        margin: 88px auto auto;
        width: min(640px, calc(100vw - 32px));
        max-height: calc(100dvh - 120px);
        border: 0;
        padding: 0;
        border-radius: var(--r-lg);
        background: var(--surface);
        color: var(--ink);
        box-shadow: var(--e-3);
        overflow: hidden;
    }

    .search::backdrop {
        background: var(--scrim);
        backdrop-filter: blur(6px);
    }

    @media (prefers-reduced-motion: no-preference) {
        .search[open] {
            animation: dialog-in var(--dur-3) var(--ease);
        }
    }

    @keyframes dialog-in {
        from {
            opacity: 0;
            translate: 0 -8px;
        }
    }

    .panel {
        display: flex;
        flex-direction: column;
        max-height: inherit;
    }

    .input {
        flex: none;
        height: 60px;
        display: flex;
        align-items: center;
        gap: var(--s-3);
        padding: 0 var(--s-4) 0 var(--s-5);
        border-bottom: 1px solid var(--line);
        color: var(--ink-3);
    }

    input {
        flex: 1;
        min-width: 0;
        height: 100%;
        border: 0;
        background: none;
        color: var(--ink);
        font-size: 20px;
        caret-color: var(--accent);
    }

    input:focus {
        outline: none;
    }

    input::-webkit-search-cancel-button {
        display: none;
    }

    .input:focus-within {
        box-shadow: inset 0 -2px 0 var(--accent);
    }

    .cancel {
        display: none;
        height: 36px;
        padding: 0 14px;
        border-radius: var(--r-pill);
        background: var(--sunken);
        color: var(--ink);
        font-size: 15px;
        font-weight: 500;
    }

    .results {
        overflow-y: auto;
        padding: 0 var(--s-2);
    }

    .results > div {
        padding: var(--s-2) 0;
    }

    h2 {
        font-size: 12px;
        line-height: 16px;
        font-weight: 600;
        color: var(--ink-3);
        padding: var(--s-2) var(--s-3);
    }

    .status {
        font-size: 14px;
        color: var(--ink-3);
    }

    .status.shown {
        padding: var(--s-4) var(--s-5);
    }

    .foot {
        flex: none;
        display: flex;
        gap: var(--s-5);
        padding: 10px var(--s-5);
        border-top: 1px solid var(--line);
        font-size: 12px;
        color: var(--ink-3);
    }

    .foot span {
        display: inline-flex;
        align-items: center;
        gap: 4px;
    }

    @media (max-width: 733px) {
        .search {
            margin: 0;
            width: 100vw;
            max-width: none;
            height: 100dvh;
            max-height: none;
            border-radius: 0;
        }

        .esc,
        .foot {
            display: none;
        }

        .cancel {
            display: inline-flex;
            align-items: center;
        }

        .results {
            flex: 1;
        }
    }
</style>
