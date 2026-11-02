<script lang="ts">
    // The search field in the header, the menu sheet and the 404 page. It opens the search dialog.
    import Icon from "../ui/Icon.svelte";
    import { search } from "#lib/state.svelte.ts";

    let { size = "md", keys = true }: { size?: "md" | "lg" | "block"; keys?: boolean } = $props();
</script>

<button
    type="button"
    class={["search-field", size]}
    aria-keyshortcuts="Control+K Meta+K /"
    onclick={() => (search.open = true)}
>
    <Icon name="search" size={size === "lg" ? 20 : 16} />
    <span>Search the docs</span>
    {#if keys}<span class="keys" aria-hidden="true"><kbd class="kbd">Ctrl</kbd><kbd class="kbd">K</kbd></span>{/if}
</button>

<style>
    .search-field {
        display: flex;
        align-items: center;
        gap: var(--s-2);
        height: 34px;
        width: 220px;
        padding: 0 6px 0 12px;
        border-radius: var(--r-sm);
        background: var(--sunken);
        color: var(--ink-3);
        font-size: 14px;
        white-space: nowrap;
        transition: box-shadow var(--dur-1) var(--ease);
    }

    .search-field:hover {
        box-shadow: inset 0 0 0 1px var(--line);
    }

    .keys {
        margin-left: auto;
        display: flex;
        gap: 3px;
    }

    .block {
        width: 100%;
        height: 44px;
    }

    .lg {
        width: min(480px, 100%);
        height: 52px;
        padding: 0 20px;
        font-size: 17px;
        border-radius: var(--r-pill);
        gap: var(--s-3);
    }

    @media (max-width: 1199px) {
        .md {
            width: 170px;
        }

        .md .keys {
            display: none;
        }
    }
</style>
