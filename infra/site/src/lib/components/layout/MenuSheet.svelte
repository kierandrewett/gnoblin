<script lang="ts">
    // The phone and tablet menu: a modal sheet from the right (native <dialog>: focus stays inside, Esc closes it
    // and focus returns to the menu button). On docs pages it holds the docs navigation.
    import { afterNavigate } from "$app/navigation";
    import { PROJECT_LINKS, SITE_NAV, type SiteArea } from "#lib/site.ts";
    import { search } from "#lib/state.svelte.ts";
    import SearchField from "../search/SearchField.svelte";
    import IconButton from "../ui/IconButton.svelte";
    import ThemeToggle from "../ui/ThemeToggle.svelte";
    import SideNav from "./SideNav.svelte";

    let {
        open = $bindable(false),
        docsSlug = null,
        area = null,
    }: { open?: boolean; docsSlug?: string | null; area?: SiteArea | null } = $props();

    let dialog: HTMLDialogElement;

    $effect(() => {
        if (open && !dialog.open) dialog.showModal();
        else if (!open && dialog.open) dialog.close();
    });
    // Opening search from the sheet replaces it.
    $effect(() => {
        if (search.open) open = false;
    });
    afterNavigate(() => (open = false));
</script>

<!-- A click on the backdrop (the dialog element itself, outside the panel) closes the sheet; Esc is native. -->
<dialog
    bind:this={dialog}
    class="sheet"
    aria-label="Menu"
    onclose={() => (open = false)}
    onclick={(event) => {
        if (event.target === dialog) open = false;
    }}
>
    <div class="panel">
        <div class="head">
            <span class="title">{docsSlug !== null ? "Docs" : "Menu"}</span>
            <IconButton icon="x" label="Close menu" onclick={() => (open = false)} />
        </div>
        <div class="search"><SearchField size="block" keys={false} /></div>
        <nav aria-label="Main" class="site">
            <ul role="list">
                {#each SITE_NAV as item (item.href)}
                    <li><a href={item.href} aria-current={item.area === area ? "page" : undefined}>{item.label}</a></li>
                {/each}
            </ul>
        </nav>
        {#if docsSlug !== null}<SideNav current={docsSlug} sheet />{/if}
        <div class="foot">
            <nav aria-label="Project" class="proj">
                {#each PROJECT_LINKS as link (link.label)}<a href={link.href}>{link.label}</a>{/each}
            </nav>
            <ThemeToggle />
        </div>
    </div>
</dialog>

<style>
    .sheet {
        margin: 0 0 0 auto;
        width: min(340px, 100vw);
        max-width: none;
        height: 100dvh;
        max-height: none;
        border: 0;
        padding: 0;
        background: var(--bg);
        color: var(--ink);
        box-shadow: var(--e-3);
    }

    .sheet::backdrop {
        background: var(--scrim);
    }

    @media (prefers-reduced-motion: no-preference) {
        .sheet,
        .sheet::backdrop {
            transition:
                translate var(--dur-3) var(--ease),
                opacity var(--dur-3) var(--ease),
                overlay var(--dur-3) allow-discrete,
                display var(--dur-3) allow-discrete;
        }

        .sheet:not([open]) {
            translate: 100% 0;
        }

        .sheet:not([open])::backdrop {
            opacity: 0;
        }

        @starting-style {
            .sheet[open] {
                translate: 100% 0;
            }

            .sheet[open]::backdrop {
                opacity: 0;
            }
        }
    }

    .panel {
        min-height: 100%;
        display: flex;
        flex-direction: column;
    }

    .head {
        position: sticky;
        top: 0;
        z-index: 1;
        display: flex;
        align-items: center;
        justify-content: space-between;
        height: 56px;
        padding: 0 4px 0 20px;
        background: var(--bg);
    }

    .title {
        font-size: 17px;
        font-weight: 600;
    }

    .search {
        padding: 0 16px 12px;
    }

    .site {
        padding: 0 var(--s-4) var(--s-3);
        border-bottom: 1px solid var(--line);
        margin-bottom: var(--s-3);
    }

    .site a {
        display: flex;
        align-items: center;
        min-height: 44px;
        padding: 0 10px;
        border-radius: var(--r-sm);
        font-size: 15px;
        font-weight: 500;
    }

    .site a:hover,
    .site a[aria-current] {
        background: var(--sunken);
    }

    .foot {
        margin-top: auto;
        display: flex;
        align-items: center;
        justify-content: space-between;
        padding: var(--s-3) var(--s-4) var(--s-3) var(--s-5);
        border-top: 1px solid var(--line);
    }

    .proj {
        display: flex;
        gap: var(--s-5);
        font-size: 14px;
        color: var(--ink-2);
    }

    .proj a {
        min-height: 44px;
        display: inline-flex;
        align-items: center;
    }
</style>
