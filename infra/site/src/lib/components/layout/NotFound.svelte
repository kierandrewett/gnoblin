<script lang="ts">
    import { docsHref } from "#lib/nav.ts";
    import SearchField from "../search/SearchField.svelte";
    import MoreLink from "../ui/MoreLink.svelte";
</script>

<section class="wrap nf">
    <p class="status">404</p>
    <h1>This page is not here.</h1>
    <p class="sub">Search the docs for it, or start from one of these pages.</p>
    <div class="field"><SearchField size="lg" keys={false} /></div>
    <div class="links">
        <MoreLink href="/install">Install</MoreLink>
        <MoreLink href={docsHref("config")}>Config API</MoreLink>
        <MoreLink href={docsHref("bring-your-own-shell")}>Choose a shell</MoreLink>
    </div>
</section>

<style>
    .nf {
        display: flex;
        flex-direction: column;
        align-items: center;
        text-align: center;
        padding-top: 180px;
        padding-bottom: 220px;
    }

    .status {
        font-size: 16px;
        line-height: 24px;
        font-weight: 600;
        color: var(--ink-3);
        margin-bottom: var(--s-4);
    }

    h1 {
        font-size: var(--t-h1-size);
        line-height: var(--t-h1-lh);
        font-weight: 700;
        text-wrap: balance;
    }

    .sub {
        font-size: var(--t-lead-size);
        line-height: var(--t-lead-lh);
        color: var(--ink-3);
        max-width: 640px;
        margin-top: var(--s-4);
    }

    .field {
        display: flex;
        justify-content: center;
        width: 100%;
        margin-top: var(--s-6);
    }

    .links {
        display: flex;
        flex-wrap: wrap;
        justify-content: center;
        gap: var(--s-3) var(--s-6);
        margin-top: var(--s-5);
    }

    @media (max-width: 733px) {
        .nf {
            padding-top: 96px;
            padding-bottom: 120px;
        }
    }
</style>
