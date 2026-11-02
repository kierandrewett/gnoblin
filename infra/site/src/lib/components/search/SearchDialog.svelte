<script lang="ts">
    // Search the docs: Ctrl+K, Cmd+K or "/" opens it, Esc closes it and returns focus. Arrow keys move through the
    // results, Enter opens one. Results come from the Pagefind index that `yarn build` writes to /pagefind/, which
    // is loaded when the browser is idle after the page loads, or earlier when the search button is touched.
    import { goto } from "$app/navigation";
    import { onMount } from "svelte";
    import { search } from "#lib/state.svelte.ts";
    import Icon from "../ui/Icon.svelte";
    import { SUGGESTIONS, type Group } from "./hit.ts";
    import { find, warm, type Found } from "./pagefind.ts";
    import SearchResult from "./SearchResult.svelte";

    /** Pages whose data is fetched for the first screen, and for each "Show more". */
    const PAGE = 6;
    /** Typing pause before a query runs. The first query after an empty box runs at once. */
    const DEBOUNCE_MS = 40;

    let dialog: HTMLDialogElement;
    let input = $state<HTMLInputElement>();
    let query = $state("");
    // The query the shown results are for. It trails `query` while a search runs.
    let shown = $state("");
    let groups = $state.raw<Group[]>([]);
    let total = $state(0);
    let selected = $state(0);
    let status = $state<"idle" | "searching" | "done" | "unavailable">("idle");

    const rows = $derived(groups.flatMap((group) => group.rows));
    // Each group with the index of its first row in the list of options.
    const entries = $derived.by(() => {
        let start = 0;
        return groups.map((group) => {
            const entry = { group, start };
            start += group.rows.length;
            return entry;
        });
    });
    const hasMore = $derived(groups.length < total);
    const options = $derived(rows.length + (hasMore ? 1 : 0));
    const summary = $derived(
        total === 0
            ? ""
            : `${total} ${total === 1 ? "result" : "results"}${hasMore ? `, showing ${groups.length}` : ""}`,
    );

    let found: Found | null = null;
    let token = 0;
    let timer: ReturnType<typeof setTimeout> | undefined;
    // Enter pressed before the results for the typed query arrived: open the first one when they do.
    let openFirst = false;

    onMount(() => {
        // The built site has the index; `yarn dev` does not. Skip the preload when the reader saves data.
        const connection = (navigator as Navigator & { connection?: { saveData?: boolean } }).connection;
        if (import.meta.env.DEV || connection?.saveData) return;
        const preload = () => void warm();
        if ("requestIdleCallback" in window) {
            const handle = requestIdleCallback(preload, { timeout: 2000 });
            return () => cancelIdleCallback(handle);
        }
        const handle = setTimeout(preload, 1500);
        return () => clearTimeout(handle);
    });

    $effect(() => {
        if (search.open && !dialog.open) {
            dialog.showModal();
            input?.select();
            void warm();
        } else if (!search.open && dialog.open) {
            dialog.close();
        }
    });

    function onInput() {
        clearTimeout(timer);
        const q = query.trim();
        if (!q) {
            token++;
            openFirst = false;
            found = null;
            groups = [];
            total = 0;
            shown = "";
            status = "idle";
            return;
        }
        timer = setTimeout(
            () => {
                timer = undefined;
                void run(q);
            },
            rows.length ? DEBOUNCE_MS : 0,
        );
    }

    async function run(q: string) {
        const mine = ++token;
        status = "searching";
        const next = await find(q);
        if (mine !== token) return;
        if (!next) {
            openFirst = false;
            status = "unavailable";
            return;
        }
        const first = await next.more(PAGE);
        // A later query started while this one was fetching its pages: drop this answer.
        if (mine !== token) return;
        found = next;
        groups = first;
        total = next.total;
        shown = q;
        selected = 0;
        status = "done";
        if (openFirst) {
            openFirst = false;
            pick(0);
        }
    }

    async function showMore() {
        const mine = token;
        const next = await found?.more(PAGE);
        if (!next?.length || mine !== token) return;
        selected = rows.length;
        groups = [...groups, ...next];
    }

    function pick(index: number) {
        const row = rows[index];
        if (!row) {
            if (hasMore && index === rows.length) void showMore();
            return;
        }
        search.open = false;
        void goto(row.url);
    }

    function suggest(text: string) {
        query = text;
        input?.focus();
        onInput();
    }

    function onKey(event: KeyboardEvent) {
        if (event.key === "ArrowDown" || event.key === "ArrowUp") {
            event.preventDefault();
            if (!options) return;
            const step = event.key === "ArrowDown" ? 1 : -1;
            selected = (selected + step + options) % options;
            document.getElementById(`search-hit-${selected}`)?.scrollIntoView({ block: "nearest" });
        } else if (event.key === "Enter") {
            event.preventDefault();
            const q = query.trim();
            if (q === shown) pick(selected);
            else if (q) {
                openFirst = true;
                if (timer !== undefined) {
                    clearTimeout(timer);
                    timer = undefined;
                    void run(q);
                }
            }
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
                oninput={onInput}
                onkeydown={onKey}
                type="search"
                placeholder="Search the docs"
                aria-label="Search the docs"
                role="combobox"
                aria-expanded={options > 0}
                aria-controls="search-results"
                aria-activedescendant={options ? `search-hit-${selected}` : undefined}
                aria-autocomplete="list"
                autocomplete="off"
                spellcheck="false"
            />
            <kbd class="kbd esc">Esc</kbd>
            <button type="button" class="cancel" onclick={() => (search.open = false)}>Cancel</button>
        </div>
        <div class="scroll">
            <p class="summary" aria-live="polite">
                {#if status === "done"}{summary}{/if}
            </p>
            <div
                class="results"
                id="search-results"
                role="listbox"
                aria-label="Results"
                aria-busy={status === "searching"}
                data-query={shown}
            >
                {#each entries as { group, start } (group.url)}
                    <div class="group" role="group" aria-label={group.title}>
                        {#each group.rows as row, ri (row.url)}
                            <SearchResult
                                {row}
                                query={shown}
                                id="search-hit-{start + ri}"
                                selected={start + ri === selected}
                                onselect={() => pick(start + ri)}
                                onhover={() => (selected = start + ri)}
                            />
                        {/each}
                    </div>
                {/each}
                {#if hasMore}
                    <div
                        id="search-hit-{rows.length}"
                        class="more"
                        role="option"
                        aria-selected={selected === rows.length}
                        tabindex="-1"
                        onclick={showMore}
                        onkeydown={(event) => event.key === "Enter" && void showMore()}
                        onmousemove={() => (selected = rows.length)}
                    >
                        Show {Math.min(PAGE, total - groups.length)} more
                    </div>
                {/if}
            </div>
            {#if status === "unavailable"}
                <p class="note">Search works in the built site. Run yarn build, then yarn preview.</p>
            {:else if status === "idle" || (status === "done" && !total)}
                <div class="note">
                    {#if status === "done"}
                        <p>No results for “{shown}”. Check the spelling or use fewer words.</p>
                    {/if}
                    <p class="try">
                        <span>Try</span>
                        {#each SUGGESTIONS as text (text)}
                            <button type="button" class="chip" onclick={() => suggest(text)}>{text}</button>
                        {/each}
                    </p>
                </div>
            {/if}
        </div>
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
        width: min(680px, calc(100vw - 32px));
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

    .scroll {
        overflow-y: auto;
        overscroll-behavior: contain;
        padding: 0 var(--s-2) var(--s-2);
    }

    .summary {
        padding: var(--s-3) var(--s-3) var(--s-1);
        font-size: 12px;
        line-height: 16px;
        font-weight: 600;
        color: var(--ink-3);
    }

    .summary:empty {
        display: none;
    }

    .group {
        padding: var(--s-1) 0;
    }

    .group + .group {
        border-top: 1px solid var(--line);
    }

    .more {
        margin-top: var(--s-1);
        padding: 10px var(--s-3) 10px calc(32px + var(--s-3) * 2);
        border-radius: var(--r-sm);
        font-size: 14px;
        font-weight: 500;
        color: var(--accent);
        cursor: pointer;
    }

    .more[aria-selected="true"] {
        background: var(--accent);
        color: var(--on-accent);
    }

    .note {
        padding: var(--s-4) var(--s-3);
        font-size: 14px;
        line-height: 22px;
        color: var(--ink-3);
    }

    .try {
        display: flex;
        flex-wrap: wrap;
        align-items: center;
        gap: var(--s-2);
    }

    .note p + .try {
        margin-top: var(--s-3);
    }

    .chip {
        height: 30px;
        padding: 0 var(--s-3);
        border-radius: var(--r-pill);
        background: var(--sunken);
        color: var(--ink);
        font-size: 14px;
    }

    .chip:hover {
        box-shadow: inset 0 0 0 1px var(--line-strong);
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

        .scroll {
            flex: 1;
        }
    }
</style>
