<script lang="ts">
    // A full-width page section. Pages alternate plain and tinted bands, with a 1 px line between them; the
    // content fades up as the band enters the view (see base.css).
    import type { Snippet } from "svelte";

    let { tint = false, label, children }: { tint?: boolean; label?: string; children: Snippet } = $props();
</script>

<section class={["band", tint && "tint"]} aria-label={label}>
    <div class="wrap band-motion">{@render children()}</div>
</section>

<style>
    .band {
        padding: var(--s-9) 0;
        border-top: 1px solid var(--line);
    }

    .tint {
        background: var(--bg-alt);
    }

    @media (max-width: 733px) {
        .band {
            padding: var(--s-7) 0;
        }
    }
</style>
