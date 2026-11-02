<script lang="ts">
    // The site header: mark and name, the main nav, search, Source, Issues and Releases, and the appearance toggle.
    // It is the page colour until the page scrolls, then gains a rule and a blur (CSS scroll-driven, skipped with
    // reduced motion). Below 1024 px it shows the mark, search and a menu button that opens MenuSheet.
    import { PROJECT_LINKS, SITE_NAV, type SiteArea } from "#lib/site.ts";
    import { search } from "#lib/state.svelte.ts";
    import SearchField from "../search/SearchField.svelte";
    import Icon from "../ui/Icon.svelte";
    import IconButton from "../ui/IconButton.svelte";
    import Mark from "../ui/Mark.svelte";
    import ThemeToggle from "../ui/ThemeToggle.svelte";
    import MenuSheet from "./MenuSheet.svelte";

    let {
        area = null,
        variant = "page",
        docsSlug = null,
    }: {
        area?: SiteArea | null;
        /** page: centred and flush; docs: full width with a rule; clear: no fill until scrolled (Bingux). */
        variant?: "page" | "docs" | "clear";
        docsSlug?: string | null;
    } = $props();

    let menuOpen = $state(false);
</script>

<header class={["topbar", variant]}>
    <div class={["inner", variant !== "docs" && "wrap"]}>
        <a class="brand" href="/"><Mark height={26} /><span>Gnoblin</span></a>
        <nav class="site" aria-label="Main">
            {#each SITE_NAV as item (item.href)}
                <a href={item.href} aria-current={item.area === area ? "page" : undefined}>{item.label}</a>
            {/each}
        </nav>
        <div class="tools">
            <SearchField />
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
    .topbar {
        position: sticky;
        top: 0;
        z-index: 10;
        height: var(--topbar-h);
        background: var(--bg);
        color: var(--ink);
    }

    .topbar.docs {
        box-shadow: inset 0 -1px 0 var(--line);
    }

    .topbar.clear {
        background: transparent;
    }

    @media (prefers-reduced-motion: no-preference) {
        @supports (animation-timeline: scroll()) {
            .topbar {
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
        gap: var(--s-6);
    }

    .docs .inner {
        padding: 0 var(--s-5);
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
        gap: var(--s-6);
    }

    .site a {
        font-size: 13px;
        color: var(--ink-2);
        transition: color var(--dur-1) var(--ease);
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
        gap: var(--s-5);
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

    @media (max-width: 1199px) {
        .inner,
        .site {
            gap: var(--s-5);
        }

        .proj {
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

        .docs .inner {
            padding: 0 var(--s-2) 0 var(--page-pad);
        }

        .inner.wrap {
            padding-right: var(--s-2);
        }
    }
</style>
