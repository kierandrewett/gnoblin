<script lang="ts">
    // The methods of a class: each is a row that shows the signature each language declares and the first sentence, and opens to the
    // parameters, what it returns, the errors and an example. The row the address names (`#apply`) is open and framed.
    import { onMount } from "svelte";
    import type { MethodRow } from "#lib/reference/model.ts";

    let { rows }: { rows: MethodRow[] } = $props();
    let list: HTMLElement;

    onMount(() => {
        const open = () => {
            const target = document.getElementById(decodeURIComponent(location.hash.slice(1)));
            if (target instanceof HTMLDetailsElement && list.contains(target)) target.open = true;
        };
        open();
        window.addEventListener("hashchange", open);
        return () => window.removeEventListener("hashchange", open);
    });
</script>

<div class="ref-methods" bind:this={list}>
    {#each rows as row (row.id)}
        <details id={row.id} open={row.open}>
            <summary>
                <h3 class="sig">{@html row.sigHtml}</h3>
                {#if row.summaryHtml}<p class="sum">{@html row.summaryHtml}</p>{/if}
                <a class="permalink" href="#{row.id}" aria-label="Link to {row.id}"><span aria-hidden="true">#</span></a
                >
            </summary>
            <div class="body">{@html row.bodyHtml}</div>
        </details>
    {/each}
</div>
