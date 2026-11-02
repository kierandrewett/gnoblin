<script lang="ts">
    import Icon from "../ui/Icon.svelte";

    let {
        href,
        glyph,
        name,
        detail,
        action = "Guide",
        quiet = false,
    }: { href: string; glyph: string; name: string; detail: string; action?: string; quiet?: boolean } = $props();
</script>

<a class={["card", quiet && "quiet"]} {href}>
    <span class="glyph"><Icon name={glyph} size={40} /></span>
    <span class="name">{name}</span>
    <span class="detail">{detail}</span>
    <span class="more">{action}<Icon name="chevron-right" size={14} /></span>
</a>

<style>
    .card {
        display: flex;
        flex-direction: column;
        align-items: flex-start;
        gap: var(--s-1);
        min-height: 168px;
        padding: var(--s-5);
        border-radius: var(--r-md);
        background: var(--surface);
        box-shadow: inset 0 0 0 1px var(--line);
        transition: box-shadow var(--dur-1) var(--ease);
    }

    .card:hover {
        box-shadow: inset 0 0 0 1px var(--accent);
    }

    .quiet {
        background: transparent;
    }

    .glyph {
        color: var(--ink);
        margin-bottom: auto;
        padding-bottom: var(--s-3);
    }

    .name {
        font-size: 24px;
        line-height: 30px;
        font-weight: 600;
    }

    .detail {
        font-size: 14px;
        line-height: 20px;
        color: var(--ink-3);
    }

    .more {
        display: inline-flex;
        align-items: center;
        gap: 4px;
        margin-top: var(--s-1);
        font-size: 15px;
        color: var(--accent);
    }

    .card:hover .more {
        text-decoration: underline;
        text-underline-offset: 4px;
    }

    @media (max-width: 733px) {
        .card {
            min-height: 156px;
        }

        .name {
            font-size: 20px;
            line-height: 26px;
        }
    }
</style>
