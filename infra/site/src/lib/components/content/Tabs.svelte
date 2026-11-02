<script lang="ts">
    // Segmented control over panels (WAI-ARIA tabs): arrow keys, Home and End move between tabs; the panel
    // cross-fades when it changes.
    import type { Snippet } from "svelte";

    let { items, label, panel }: { items: string[]; label: string; panel: Snippet<[number]> } = $props();

    const id = $props.id();
    let selected = $state(0);
    const tabs: HTMLButtonElement[] = [];

    function onKey(event: KeyboardEvent) {
        const last = items.length - 1;
        const next = {
            ArrowRight: selected === last ? 0 : selected + 1,
            ArrowLeft: selected === 0 ? last : selected - 1,
            Home: 0,
            End: last,
        }[event.key];
        if (next === undefined) return;
        event.preventDefault();
        selected = next;
        tabs[next]?.focus();
    }
</script>

<div class="tabs">
    <div class="seg" role="tablist" aria-label={label}>
        {#each items as item, i (item)}
            <button
                type="button"
                role="tab"
                id="{id}-tab-{i}"
                aria-selected={i === selected}
                aria-controls="{id}-panel"
                tabindex={i === selected ? 0 : -1}
                bind:this={tabs[i]}
                onclick={() => (selected = i)}
                onkeydown={onKey}>{item}</button
            >
        {/each}
    </div>
    {#key selected}
        <div class="panel" role="tabpanel" id="{id}-panel" aria-labelledby="{id}-tab-{selected}">
            {@render panel(selected)}
        </div>
    {/key}
</div>

<style>
    .tabs {
        display: flex;
        flex-direction: column;
        gap: var(--s-3);
    }

    .seg {
        align-self: flex-start;
    }

    @media (prefers-reduced-motion: no-preference) {
        .panel {
            animation: panel-in var(--dur-2) var(--ease);
        }
    }

    @keyframes panel-in {
        from {
            opacity: 0;
        }
    }
</style>
