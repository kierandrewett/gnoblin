<script lang="ts">
    // A row of install guide links with each system's mark (Simple Icons, in currentColor).
    import { SYSTEMS } from "#lib/site.ts";
    import Icon from "../ui/Icon.svelte";
</script>

<nav class="distros" aria-label="Install guides">
    {#each SYSTEMS as system (system.href)}
        <a href={system.href}><Icon name={system.glyph || "code"} size={18} /><span>{system.short}</span></a>
    {/each}
</nav>

<style>
    .distros {
        display: flex;
        flex-wrap: wrap;
        gap: 8px 22px;
    }

    a {
        display: inline-flex;
        align-items: center;
        gap: 7px;
        min-height: 32px;
        font-size: 14px;
        font-weight: 500;
        color: var(--ink-2);
    }

    a span {
        text-decoration: underline;
        text-decoration-color: var(--line);
        text-underline-offset: 3px;
    }

    a:hover {
        color: var(--accent);
    }

    a:hover span {
        text-decoration-color: currentColor;
    }

    @media (max-width: 733px) {
        a {
            min-height: 44px;
        }
    }
</style>
