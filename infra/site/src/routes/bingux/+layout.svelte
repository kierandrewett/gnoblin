<script lang="ts">
    // The Bingux page body uses Bingux's own design system. data-theme="bingux" scopes its tokens and components
    // (src/lib/bingux/bingux.css) to this element; the Gnoblin header and footer sit outside it.
    import "#lib/bingux/bingux.css";
    import type { Snippet } from "svelte";

    let { children }: { children: Snippet } = $props();
</script>

<div data-theme="bingux">{@render children()}</div>
