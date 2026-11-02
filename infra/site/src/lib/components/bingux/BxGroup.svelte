<script lang="ts">
    // Bingux boxed list (crates/ui group): rows in one rounded card, separated by hairlines.
    import type { Snippet } from "svelte";

    let { title = "", children }: { title?: string; children: Snippet } = $props();
</script>

<section>
    {#if title}<h3 class="bhead">{title}</h3>{/if}
    <div class="bgroup">{@render children()}</div>
</section>
