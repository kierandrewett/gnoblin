<script lang="ts">
    // The site header, the same on every page: mark and name, the main nav, search, Source, Issues and Releases,
    // and the appearance toggle. From 1280 px it sits on the docs page grid (the mark over the navigation, the site
    // links and search over the centred 720 px column, the project links over the outline). It is the page colour
    // until the page scrolls, then gains a rule and a blur (CSS scroll-driven; with reduced motion the rule is
    // always there). Below 1024 px it shows the mark, search and a menu button that opens MenuSheet.
    import { PROJECT_LINKS, SITE_NAV, type SiteArea } from "#lib/site.ts";
    import { search } from "#lib/state.svelte.ts";
    import SearchField from "../search/SearchField.svelte";
    import Icon from "../ui/Icon.svelte";
    import IconButton from "../ui/IconButton.svelte";
    import Mark from "../ui/Mark.svelte";
    import ThemeToggle from "../ui/ThemeToggle.svelte";
    import MenuSheet from "./MenuSheet.svelte";

    let { area = null, docsSlug = null }: { area?: SiteArea | null; docsSlug?: string | null } = $props();

    let menuOpen = $state(false);
</script>

<header class="topbar">
    <div class="inner">
        <a class="brand" href="/"><Mark height={26} /><span>Gnoblin</span></a>
        <nav class="site" aria-label="Main">
            {#each SITE_NAV as item (item.href)}
                <a href={item.href} data-label={item.label} aria-current={item.area === area ? "page" : undefined}
                    >{item.label}</a
                >
            {/each}
        </nav>
        <div class="tools">
            <SearchField />
            <div class="end">
                <nav class="proj" aria-label="Project">
                    {#each PROJECT_LINKS as link (link.label)}
                        <a href={link.href}
                            >{#if link.icon}<Icon name={link.icon} size={15} />{/if}{link.label}</a
                        >
                    {/each}
                </nav>
                <span class="sep" aria-hidden="true"></span>
                <ThemeToggle />
            </div>
        </div>
        <div class="compact">
            <IconButton icon="search" label="Search the docs" onclick={() => (search.open = true)} />
            <IconButton
                icon="menu"
                label="Open menu"
                aria-haspopup="dialog"
                aria-expanded={menuOpen}
                onclick={() => (menuOpen = true)}
            />
        </div>
    </div>
</header>
<MenuSheet bind:open={menuOpen} {docsSlug} {area} />

<style>
    /* The rule is an inset shadow, so it never changes the height. */
    .topbar {
        position: sticky;
        top: 0;
        z-index: 10;
        height: var(--topbar-h);
        background: var(--bg);
        color: var(--ink);
        box-shadow: inset 0 -1px 0 var(--line);
    }

    @media (prefers-reduced-motion: no-preference) {
        @supports (animation-timeline: scroll()) {
            .topbar {
                box-shadow: none;
                animation: scrolled linear both;
                animation-timeline: scroll(root);
                animation-range: 0 8px;
            }
        }
    }

    @keyframes scrolled {
        to {
            background: var(--bar);
            backdrop-filter: saturate(180%) blur(16px);
            box-shadow: inset 0 -1px 0 var(--line);
        }
    }

    .inner {
        height: 100%;
        display: flex;
        align-items: center;
        gap: var(--s-5);
        padding: 0 var(--s-5);
    }

    .end {
        display: contents;
    }

    .brand {
        display: inline-flex;
        align-items: center;
        gap: 9px;
        font-weight: 600;
        font-size: 19px;
        line-height: 1;
    }

    .site {
        display: flex;
        gap: var(--s-5);
    }

    .site a {
        display: inline-flex;
        flex-direction: column;
        font-size: 13px;
        color: var(--ink-2);
        transition: color var(--dur-1) var(--ease);
    }

    /* Each link reserves the width of its bold form, so marking the current area moves nothing. */
    .site a::after {
        content: attr(data-label);
        height: 0;
        overflow: hidden;
        visibility: hidden;
        font-weight: 600;
    }

    .site a:hover,
    .site a[aria-current] {
        color: var(--ink);
    }

    .site a[aria-current] {
        font-weight: 600;
    }

    .tools {
        margin-left: auto;
        display: flex;
        align-items: center;
        gap: var(--s-4);
    }

    .proj {
        display: flex;
        align-items: center;
        gap: var(--s-4);
        font-size: 13px;
        color: var(--ink-2);
    }

    .proj a {
        display: inline-flex;
        align-items: center;
        gap: 6px;
    }

    .proj a:hover {
        color: var(--ink);
        text-decoration: underline;
        text-underline-offset: 3px;
    }

    .sep {
        width: 1px;
        height: 18px;
        background: var(--line);
    }

    .compact {
        display: none;
        margin-left: auto;
        gap: var(--s-1);
    }

    /* From 1280 px: the docs page's three columns. */
    @media (min-width: 1280px) {
        .inner {
            display: grid;
            grid-template-columns:
                minmax(0, 1fr) min(var(--measure), 100% - 2 * var(--page-pad))
                minmax(0, 1fr);
            column-gap: var(--s-8);
            padding: 0;
        }

        .brand {
            grid-column: 1;
            justify-self: end;
            width: var(--nav-w);
        }

        .site {
            grid-area: 1 / 2;
            justify-self: start;
        }

        .tools {
            display: contents;
        }

        .tools > :global(.search-field) {
            grid-area: 1 / 2;
            justify-self: end;
        }

        .end {
            grid-column: 3;
            display: flex;
            align-items: center;
            gap: var(--s-4);
        }
    }

    @media (max-width: 1023px) {
        .site,
        .tools {
            display: none;
        }

        .compact {
            display: flex;
        }

        .inner {
            padding: 0 var(--s-2) 0 var(--page-pad);
        }
    }
</style>
