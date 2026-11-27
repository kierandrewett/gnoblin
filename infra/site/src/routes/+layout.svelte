<script lang="ts">
    import "#lib/styles/tokens.css";
    import "#lib/styles/base.css";
    import "#lib/styles/content.css";
    import type { Snippet } from "svelte";
    import { page } from "$app/state";
    import { FAVICON_URL } from "#lib/brand/index.ts";
    import Footer from "#lib/components/layout/Footer.svelte";
    import ReferenceTree from "#lib/components/reference/ReferenceTree.svelte";
    import TopBar from "#lib/components/layout/TopBar.svelte";
    import SearchDialog from "#lib/components/search/SearchDialog.svelte";
    import type { DocsPage } from "#lib/docs.ts";
    import type { SiteArea } from "#lib/site.ts";
    import { announcer } from "#lib/state.svelte.ts";

    let { children }: { children: Snippet } = $props();

    // The home page and the docs pages share the header bar and the centred one-line footer.
    const route = $derived(page.route.id ?? "");
    const docsPage = $derived<DocsPage | undefined>(route.startsWith("/docs") ? page.data.page : undefined);
    const isReference = $derived(route.startsWith("/reference"));
    const area = $derived.by((): SiteArea | null => {
        if (route === "/bingux") return "shell";
        if (isReference) return "reference";
        if (!docsPage) return null;
        return docsPage.section ?? "start";
    });
</script>

<svelte:head>
    <link rel="icon" type="image/svg+xml" href={FAVICON_URL} />
</svelte:head>

<a class="skip-link" href="#main">Skip to content</a>
{#snippet referenceMenu()}
    <ReferenceTree nav={page.data.nav} current={page.url.pathname} sheet />
{/snippet}
<TopBar {area} docsSlug={docsPage ? docsPage.slug : null} menu={isReference ? referenceMenu : undefined} />
<main id="main" tabindex="-1">
    {@render children()}
</main>
<Footer variant={route === "/" || docsPage || isReference ? "home" : "site"} />
<SearchDialog />
<div class="visually-hidden" aria-live="polite">{announcer.message}</div>
