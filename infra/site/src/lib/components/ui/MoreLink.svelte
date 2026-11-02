<script lang="ts">
    // A text link with a chevron: "Choose a shell ›".
    import type { Snippet } from "svelte";
    import Icon from "./Icon.svelte";

    let { href, children }: { href: string; children: Snippet } = $props();
</script>

<a class="more" {href}>{@render children()}<Icon name="chevron-right" size={14} /></a>

<style>
    .more {
        display: inline-flex;
        align-items: center;
        gap: 4px;
        color: var(--accent);
        font-size: var(--t-body-size);
        line-height: var(--t-body-lh);
    }

    .more :global(svg) {
        transition: translate var(--dur-1) var(--ease);
    }

    .more:hover {
        text-decoration: underline;
        text-underline-offset: 4px;
    }

    .more:hover :global(svg) {
        translate: 2px 0;
    }

    @media (prefers-reduced-motion: reduce) {
        .more:hover :global(svg) {
            translate: none;
        }
    }
</style>
