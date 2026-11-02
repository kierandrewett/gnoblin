<script lang="ts">
    // Numbered steps in three columns (one on phones). Each step is a snippet with its heading, text and link.
    import type { Snippet } from "svelte";

    let { steps }: { steps: Snippet[] } = $props();
</script>

<ol class="steps" role="list">
    {#each steps as step, i (i)}<li>{@render step()}</li>{/each}
</ol>

<style>
    .steps {
        display: grid;
        grid-template-columns: repeat(3, 1fr);
        gap: var(--s-6);
        counter-reset: step;
    }

    li {
        counter-increment: step;
        display: flex;
        flex-direction: column;
        align-items: flex-start;
        gap: var(--s-2);
    }

    li::before {
        content: counter(step);
        width: 28px;
        height: 28px;
        border-radius: 50%;
        display: grid;
        place-items: center;
        box-shadow: inset 0 0 0 1px var(--line-strong);
        font-family: var(--font-mono);
        font-size: 13px;
        font-weight: 500;
        color: var(--ink-2);
        margin-bottom: var(--s-2);
    }

    li :global(h3) {
        font-size: var(--t-h3-size);
        line-height: var(--t-h3-lh);
        font-weight: 600;
    }

    li :global(p) {
        color: var(--ink-3);
    }

    @media (max-width: 733px) {
        .steps {
            grid-template-columns: 1fr;
            gap: var(--s-7);
        }
    }
</style>
