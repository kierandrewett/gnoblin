<script lang="ts">
    import type { Snippet } from "svelte";
    import Icon from "../ui/Icon.svelte";

    type Kind = "note" | "tip" | "warning" | "danger";
    let { kind = "note", title = "", children }: { kind?: Kind; title?: string; children: Snippet } = $props();
    const TITLES: Record<Kind, string> = { note: "Note", tip: "Tip", warning: "Warning", danger: "Danger" };
</script>

<aside class={["callout", kind !== "note" && kind]} role="note">
    <Icon name={kind} size={20} />
    <div>
        <p class="title">{title || TITLES[kind]}</p>
        {@render children()}
    </div>
</aside>
