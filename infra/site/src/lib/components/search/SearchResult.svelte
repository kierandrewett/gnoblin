<script lang="ts">
    import Icon from "../ui/Icon.svelte";
    import type { Hit } from "./hit.ts";

    let {
        hit,
        query,
        id,
        selected,
        onselect,
        onhover,
    }: { hit: Hit; query: string; id: string; selected: boolean; onselect: () => void; onhover: () => void } = $props();

    // The words of the query, marked where they appear in the title.
    const parts = $derived.by(() => {
        const words = query
            .trim()
            .split(/\s+/)
            .filter(Boolean)
            .map((w) => w.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"));
        if (!words.length) return [{ text: hit.title, mark: false }];
        const re = new RegExp(`(${words.join("|")})`, "gi");
        // split() with a capturing group puts the matches at the odd indexes.
        return hit.title
            .split(re)
            .map((text, i) => ({ text, mark: i % 2 === 1 }))
            .filter((p) => p.text);
    });
</script>

<div
    {id}
    class="result"
    role="option"
    aria-selected={selected}
    tabindex="-1"
    onclick={onselect}
    onkeydown={(event) => event.key === "Enter" && onselect()}
    onmousemove={onhover}
>
    <span class="ri"><Icon name={hit.kind === "page" ? "page" : "hash"} size={17} /></span>
    <span class="text">
        <span class="rt"
            >{#each parts as p, i (i)}{#if p.mark}<mark>{p.text}</mark>{:else}{p.text}{/if}{/each}</span
        >
        {#if hit.path}<span class="rp">{hit.path}</span>{/if}
    </span>
    <span class="enter"><Icon name="enter" size={18} /></span>
</div>

<style>
    .result {
        display: grid;
        grid-template-columns: 32px 1fr auto;
        align-items: center;
        gap: var(--s-3);
        padding: 10px var(--s-3);
        border-radius: var(--r-sm);
        cursor: pointer;
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

    .text {
        display: flex;
        flex-direction: column;
        min-width: 0;
    }

    .rt {
        font-size: 15px;
        line-height: 22px;
        font-weight: 500;
        overflow-wrap: anywhere;
    }

    .rp {
        font-size: 13px;
        line-height: 20px;
        color: var(--ink-3);
    }

    mark {
        background: none;
        color: var(--accent);
        font-weight: 600;
    }

    .enter {
        opacity: 0;
    }

    .result[aria-selected="true"] {
        background: var(--accent);
        color: var(--on-accent);
    }

    .result[aria-selected="true"] .rp,
    .result[aria-selected="true"] mark {
        color: var(--on-accent);
    }

    .result[aria-selected="true"] .ri {
        background: color-mix(in srgb, var(--on-accent) 15%, transparent);
        color: var(--on-accent);
    }

    .result[aria-selected="true"] .enter {
        opacity: 1;
    }
</style>
