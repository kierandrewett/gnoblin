<script lang="ts">
    // One option in the results: a page (title, section path, excerpt) or a heading inside a page.
    import Icon from "../ui/Icon.svelte";
    import { highlight, type Row } from "./hit.ts";

    let {
        row,
        query,
        id,
        selected,
        onselect,
        onhover,
    }: { row: Row; query: string; id: string; selected: boolean; onselect: () => void; onhover: () => void } = $props();

    const title = $derived(highlight(row.title, query));
    const trail = $derived(row.kind === "page" ? highlight(row.trail, query) : []);
</script>

<div
    {id}
    class={["result", row.kind]}
    role="option"
    aria-selected={selected}
    tabindex="-1"
    onclick={onselect}
    onkeydown={(event) => event.key === "Enter" && onselect()}
    onmousemove={onhover}
>
    <span class="ri"><Icon name={row.kind === "page" ? "page" : "hash"} size={row.kind === "page" ? 17 : 15} /></span>
    <span class="text">
        <span class="head">
            <span class="rt"
                >{#each title as p, i (i)}{#if p.mark}<mark>{p.text}</mark>{:else}{p.text}{/if}{/each}</span
            >
            {#if trail.length}
                <span class="rp"
                    >{#each trail as p, i (i)}{#if p.mark}<mark>{p.text}</mark>{:else}{p.text}{/if}{/each}</span
                >
            {/if}
        </span>
        {#if row.excerpt.length}
            <span class="ex"
                >{#each row.excerpt as p, i (i)}{#if p.mark}<mark>{p.text}</mark>{:else}{p.text}{/if}{/each}</span
            >
        {/if}
    </span>
    <span class="enter"><Icon name="enter" size={18} /></span>
</div>

<style>
    .result {
        display: grid;
        grid-template-columns: 32px 1fr auto;
        align-items: start;
        gap: var(--s-3);
        padding: 10px var(--s-3);
        border-radius: var(--r-sm);
        cursor: pointer;
    }

    .heading {
        grid-template-columns: 24px 1fr auto;
        margin-left: var(--s-6);
        padding-block: 6px;
    }

    .ri {
        width: 32px;
        height: 32px;
        border-radius: var(--r-sm);
        background: var(--sunken);
        display: grid;
        place-items: center;
        color: var(--ink-3);
    }

    .heading .ri {
        width: 24px;
        height: 24px;
        background: none;
    }

    .text {
        display: flex;
        flex-direction: column;
        min-width: 0;
    }

    .head {
        display: flex;
        flex-wrap: wrap;
        align-items: baseline;
        column-gap: var(--s-3);
    }

    .rt {
        font-size: 15px;
        line-height: 22px;
        font-weight: 500;
        overflow-wrap: anywhere;
    }

    .heading .rt {
        font-size: 14px;
        line-height: 20px;
    }

    .rp {
        font-size: 13px;
        line-height: 20px;
        color: var(--ink-3);
    }

    .ex {
        display: -webkit-box;
        -webkit-box-orient: vertical;
        -webkit-line-clamp: 2;
        line-clamp: 2;
        overflow: hidden;
        overflow-wrap: anywhere;
        font-size: 13px;
        line-height: 20px;
        color: var(--ink-2);
    }

    mark {
        background: none;
        color: var(--accent);
        font-weight: 600;
    }

    .enter {
        opacity: 0;
        align-self: center;
    }

    .result[aria-selected="true"] {
        background: var(--accent);
        color: var(--on-accent);
    }

    .result[aria-selected="true"] .rp,
    .result[aria-selected="true"] .ex,
    .result[aria-selected="true"] mark {
        color: var(--on-accent);
    }

    .result[aria-selected="true"] mark {
        text-decoration: underline;
        text-underline-offset: 2px;
    }

    .result[aria-selected="true"] .ri {
        background: color-mix(in srgb, var(--on-accent) 15%, transparent);
        color: var(--on-accent);
    }

    .result[aria-selected="true"] .enter {
        opacity: 1;
    }

    @media (max-width: 733px) {
        .heading {
            margin-left: var(--s-4);
        }
    }
</style>
