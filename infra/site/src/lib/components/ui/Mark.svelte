<script lang="ts">
    // The brand mark from src/lib/brand/mark.svg, inline so it takes the colour of its context. `height` sets the
    // size; the width follows the mark's viewBox, so a wider concept can be swapped in without changes here.
    import { MARK_SVG } from "#lib/brand/index.ts";

    let { height = 26 }: { height?: number } = $props();
    const [, , w = 1, h = 1] = (/viewBox="([^"]+)"/.exec(MARK_SVG)?.[1] ?? "0 0 1 1").split(/[\s,]+/).map(Number);
    // Only the root <svg>'s own size is replaced; shapes inside keep their width and height.
    const svg = $derived(
        MARK_SVG.replace(/<svg\b[^>]*>/, (tag) =>
            tag
                .replace(/\s(width|height)="[^"]*"/g, "")
                .replace(
                    /^<svg/,
                    `<svg width="${Math.round((height * w) / h)}" height="${height}" aria-hidden="true" focusable="false"`,
                ),
        ),
    );
</script>

<span class="mark">{@html svg}</span>

<style>
    .mark {
        display: inline-flex;
        flex: none;
        color: var(--accent);
    }
</style>
