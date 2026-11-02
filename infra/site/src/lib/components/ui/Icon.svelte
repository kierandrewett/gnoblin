<script lang="ts">
    // An icon from the sprite in static/icons.svg (scripts/build-icons.mjs). Decorative unless it has a label.
    let { name, size = 20, label = "" }: { name: string; size?: number; label?: string } = $props();
</script>

<svg
    class="icon"
    width={size}
    height={size}
    aria-hidden={label ? undefined : "true"}
    role={label ? "img" : undefined}
    aria-label={label || undefined}
>
    <use href="/icons.svg#i-{name}" />
</svg>
