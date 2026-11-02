<script lang="ts">
    // A docs page: the article (breadcrumbs, title, lede, content, edit link, previous and next) in the home
    // page's centred 720 px column, with the navigation and the sticky outline in the side margins. The outline
    // moves into the article below 1280 px, and the navigation into the menu sheet below 1024 px.
    import { codeBlocks } from "#lib/actions/code-blocks.ts";
    import Details from "#lib/components/content/Details.svelte";
    import Breadcrumbs from "#lib/components/layout/Breadcrumbs.svelte";
    import EditLink from "#lib/components/layout/EditLink.svelte";
    import OnThisPage from "#lib/components/layout/OnThisPage.svelte";
    import Pager from "#lib/components/layout/Pager.svelte";
    import SideNav from "#lib/components/layout/SideNav.svelte";
    import "#lib/styles/prose.css";
    import type { PageProps } from "./$types";

    let { data }: PageProps = $props();
    const doc = $derived(data.page);
    // API reference titles (gnoblin.configure.input.mouse) are set in monospace.
    const ident = $derived(/^[\w-]+(\.[\w-]+)+$/.test(doc.title));
    // Shown under each search result.
    const trail = $derived(doc.trail.map((t) => t.label).join(" › "));
</script>

<svelte:head>
    <title>{doc.slug === "" ? "Gnoblin documentation" : `${doc.title} · Gnoblin`}</title>
    <meta name="description" content={doc.lede.replace(/<[^>]+>/g, "").slice(0, 200)} />
</svelte:head>

<div class="docs">
    <div class="nav-col">
        <div class="nav-sticky"><SideNav current={doc.slug} /></div>
    </div>
    <article class="article" data-pagefind-body data-pagefind-meta="trail:{trail}">
        <div data-pagefind-ignore><Breadcrumbs trail={doc.trail} title={doc.title} /></div>
        <div class="prose" use:codeBlocks={doc.body}>
            <h1 class:ident data-pagefind-meta="title">{doc.title}</h1>
            {#if doc.lede}<p class="lede">{@html doc.lede}</p>{/if}
            {#if doc.headings.length}
                <div class="m-toc" data-pagefind-ignore>
                    <Details summary="On this page">
                        <ul role="list">
                            {#each doc.headings as h (h.id)}
                                <li class:sub={h.depth === 3}><a href="#{h.id}">{h.text}</a></li>
                            {/each}
                        </ul>
                    </Details>
                </div>
            {/if}
            {@html doc.body}
        </div>
        <div data-pagefind-ignore>
            <EditLink file={doc.file} />
            <Pager prev={doc.prev} next={doc.next} />
        </div>
    </article>
    <div class="toc-col" data-pagefind-ignore>
        <OnThisPage headings={doc.headings} file={doc.file} />
    </div>
</div>

<style>
    .docs {
        display: grid;
        grid-template-columns: minmax(0, 1fr) min(var(--measure), 100% - 2 * var(--page-pad)) minmax(0, 1fr);
        column-gap: var(--s-8);
        align-items: start;
    }

    /* Both side columns stretch to the article's height so their sticky children have room to stick. */
    .nav-col,
    .toc-col {
        align-self: stretch;
        padding-top: var(--s-8);
    }

    .nav-col {
        justify-self: end;
        width: var(--nav-w);
    }

    .toc-col {
        justify-self: start;
        width: var(--toc-w);
    }

    .nav-sticky {
        position: sticky;
        top: calc(var(--topbar-h) + var(--s-6));
        max-height: calc(100vh - var(--topbar-h) - var(--s-6));
        overflow-y: auto;
        overscroll-behavior: contain;
    }

    .article {
        padding: var(--s-8) 0 var(--s-8);
        min-width: 0;
    }

    .m-toc {
        display: none;
        margin: 0 0 var(--s-5);
    }

    .m-toc ul {
        padding: 0 0 0 2px;
    }

    .m-toc li {
        margin: 0;
    }

    .m-toc li + li {
        margin-top: 6px;
    }

    .m-toc li.sub {
        padding-left: var(--s-4);
    }

    @media (max-width: 1279px) {
        .docs {
            grid-template-columns: var(--nav-w) minmax(0, var(--measure));
            justify-content: center;
            padding: 0 var(--page-pad);
        }

        .nav-col {
            justify-self: stretch;
        }

        .toc-col {
            display: none;
        }

        .m-toc {
            display: block;
        }
    }

    @media (max-width: 1023px) {
        .docs {
            grid-template-columns: minmax(0, var(--measure));
        }

        .nav-col {
            display: none;
        }
    }

    @media (max-width: 733px) {
        .article {
            padding: var(--s-5) 0 var(--s-8);
        }
    }
</style>
