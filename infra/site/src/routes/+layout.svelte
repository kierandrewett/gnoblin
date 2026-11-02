<script lang="ts">
    import "#lib/styles/tokens.css";
    import "#lib/styles/base.css";
    import "#lib/styles/content.css";
    import type { Snippet } from "svelte";
    import { page } from "$app/state";
    import { FAVICON_URL } from "#lib/brand/index.ts";
    import Footer from "#lib/components/layout/Footer.svelte";
    import TopBar from "#lib/components/layout/TopBar.svelte";
    import SearchDialog from "#lib/components/search/SearchDialog.svelte";
    import type { DocsPage } from "#lib/docs.ts";
    import type { NavSection } from "#lib/nav.ts";
    import type { SiteArea } from "#lib/site.ts";
    import { announcer } from "#lib/state.svelte.ts";

    let { children }: { children: Snippet } = $props();

    const AREA_OF_SECTION: Record<NavSection["id"], SiteArea> = {
        start: "docs",
        shell: "shells",
        config: "configure",
        dev: "develop",
        contrib: "docs",
    };

    // The home page has no header bar and its own one-line footer; docs pages use the full-width ruled header.
    const route = $derived(page.route.id ?? "");
    const docsPage = $derived<DocsPage | undefined>(route.startsWith("/docs") ? page.data.page : undefined);
    const area = $derived.by((): SiteArea | null => {
        if (route === "/install") return "install";
        if (route === "/bingux") return "shells";
        if (!docsPage) return null;
        return docsPage.section ? AREA_OF_SECTION[docsPage.section] : "docs";
    });
</script>

<svelte:head>
    <link rel="icon" type="image/svg+xml" href={FAVICON_URL} />
</svelte:head>

<a class="skip-link" href="#main">Skip to content</a>
{#if route !== "/"}
    <TopBar
        {area}
        variant={docsPage ? "docs" : route === "/bingux" ? "clear" : "page"}
        docsSlug={docsPage ? docsPage.slug : null}
    />
{/if}
<main id="main" tabindex="-1">
    {@render children()}
</main>
<Footer variant={route === "/" ? "home" : "site"} />
<SearchDialog />
<div class="visually-hidden" aria-live="polite">{announcer.message}</div>
