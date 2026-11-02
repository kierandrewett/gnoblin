<script lang="ts">
    // A native <details>: keyboard and screen readers work without script. The plus turns into a cross when open.
    import type { Snippet } from "svelte";
    import Icon from "../ui/Icon.svelte";

    let { summary, open = false, children }: { summary: string; open?: boolean; children: Snippet } = $props();
</script>

<details class="details" {open}>
    <summary><span>{summary}</span><span class="plus"><Icon name="plus" size={18} /></span></summary>
    <div class="body">{@render children()}</div>
</details>
