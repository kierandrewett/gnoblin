<script lang="ts">
    type Link = { label: string; href: string } | null;
    let { prev, next }: { prev: Link; next: Link } = $props();
</script>

{#if prev || next}
    <nav class="pager" aria-label="Previous and next page">
        {#if prev}
            <a href={prev.href} rel="prev"><span class="dir">Previous</span><span class="ttl">{prev.label}</span></a>
        {:else}<span></span>{/if}
        {#if next}
            <a href={next.href} rel="next" class="next"
                ><span class="dir">Next</span><span class="ttl">{next.label}</span></a
            >
        {/if}
    </nav>
{/if}

<style>
    .pager {
        display: grid;
        grid-template-columns: 1fr 1fr;
        gap: var(--s-4);
        margin-top: var(--s-5);
    }

    a {
        display: flex;
        flex-direction: column;
        gap: 2px;
        padding: var(--s-4) var(--s-5);
        border-radius: var(--r-md);
        background: var(--bg-alt);
        transition: background var(--dur-1) var(--ease);
    }

    a:hover {
        background: var(--line);
    }

    .dir {
        font-size: 13px;
        color: var(--ink-3);
    }

    .ttl {
        font-size: 17px;
        font-weight: 600;
        color: var(--accent);
    }

    .next {
        grid-column: 2;
        text-align: right;
        align-items: flex-end;
    }

    @media (max-width: 733px) {
        .pager {
            grid-template-columns: 1fr;
        }

        .next {
            grid-column: auto;
        }
    }
</style>
