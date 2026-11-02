<script lang="ts">
    // Bingux preference row: optional leading content, a title and subtitle, and a chevron when it links somewhere.
    import type { Snippet } from "svelte";

    let {
        title,
        sub = "",
        href = "",
        leading,
    }: { title: string; sub?: string; href?: string; leading?: Snippet } = $props();
</script>

<svelte:element this={href ? "a" : "div"} class={["brow", sub && "sub"]} href={href || undefined}>
    {@render leading?.()}
    <div class="fill">
        <div class="t">{title}</div>
        {#if sub}<div class="s">{sub}</div>{/if}
    </div>
    {#if href}<svg class="ic chev" aria-hidden="true"><use href="/icons.svg#i-chevron-right" /></svg>{/if}
</svelte:element>
