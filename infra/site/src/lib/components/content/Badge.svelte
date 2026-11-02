<script lang="ts">
    import type { Snippet } from "svelte";

    let {
        kind = "",
        children,
    }: { kind?: "" | "accent" | "tip" | "warning" | "danger" | "outline"; children: Snippet } = $props();
</script>

<span class={["badge", kind]}>{@render children()}</span>
