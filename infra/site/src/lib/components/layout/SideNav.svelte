<script lang="ts">
    // The docs navigation from src/lib/nav.ts. Sections and groups open and close; the ones above the current
    // page open when it is shown. The current page has aria-current="page".
    import { docsHref, NAV, NAV_ORDER, type NavItem } from "#lib/nav.ts";
    import Icon from "../ui/Icon.svelte";

    let { current = null, sheet = false }: { current?: string | null; sheet?: boolean } = $props();

    // A section's key is its id; a group's key adds the labels down to it ("config/gnoblin.configure/Input").
    // The section and groups above the current page are open unless the reader closed them; others are closed
    // unless the reader opened them. Derived, so the prerendered page already shows the right ones.
    const pathKeys = $derived.by((): Set<string> => {
        const entry = NAV_ORDER.find((e) => e.slug === current);
        if (!entry) return new Set([NAV[0]!.id]);
        let key: string = entry.section.id;
        const keys = [key];
        for (const step of [...entry.trail.slice(1), { label: entry.label }]) {
            key = `${key}/${step.label}`;
            keys.push(key);
        }
        return new Set(keys);
    });
    let opened = $state(new Set<string>());
    let closed = $state(new Set<string>());
    const isOpen = (key: string) => opened.has(key) || (pathKeys.has(key) && !closed.has(key));

    function toggle(key: string) {
        const show = !isOpen(key);
        opened = new Set(show ? [...opened, key] : [...opened].filter((k) => k !== key));
        closed = new Set(show ? [...closed].filter((k) => k !== key) : [...closed, key]);
    }
</script>

{#snippet items(list: NavItem[], parent: string)}
    <ul role="list">
        {#each list as item (item.label)}
            {@const key = `${parent}/${item.label}`}
            {@const expanded = isOpen(key)}
            <li>
                {#if item.items}
                    <div class="row">
                        {#if item.slug !== undefined}
                            <a
                                href={docsHref(item.slug)}
                                aria-current={item.slug === current ? "page" : undefined}
                                data-sveltekit-preload-data="hover">{item.label}</a
                            >
                            <button
                                type="button"
                                class="chev"
                                aria-expanded={expanded}
                                aria-label="{expanded ? 'Hide' : 'Show'} pages in {item.label}"
                                onclick={() => toggle(key)}
                            >
                                <Icon name={expanded ? "chevron-down" : "chevron-right"} size={12} />
                            </button>
                        {:else}
                            <button type="button" class="group" aria-expanded={expanded} onclick={() => toggle(key)}>
                                {item.label}
                                <span class="chev-icon"
                                    ><Icon name={expanded ? "chevron-down" : "chevron-right"} size={12} /></span
                                >
                            </button>
                        {/if}
                    </div>
                    {#if expanded}{@render items(item.items, key)}{/if}
                {:else if item.slug !== undefined}
                    <a href={docsHref(item.slug)} aria-current={item.slug === current ? "page" : undefined}
                        >{item.label}</a
                    >
                {/if}
            </li>
        {/each}
    </ul>
{/snippet}

<nav class={["sidenav", sheet && "sheet"]} aria-label="Documentation">
    {#each NAV as section (section.id)}
        {@const expanded = isOpen(section.id)}
        <div class="sec">
            <button type="button" class="sec-h" aria-expanded={expanded} onclick={() => toggle(section.id)}>
                {section.label}
                <Icon name={expanded ? "chevron-down" : "chevron-right"} size={12} />
            </button>
            {#if expanded}{@render items(section.items, section.id)}{/if}
        </div>
    {/each}
</nav>

<style>
    .sidenav {
        padding: 0 0 var(--s-8);
    }

    .sidenav.sheet {
        padding: var(--s-1) var(--s-4) var(--s-6);
    }

    .sec {
        margin-bottom: var(--s-4);
    }

    .sec-h {
        display: flex;
        align-items: center;
        justify-content: space-between;
        width: 100%;
        min-height: 32px;
        padding: 0;
        margin-bottom: 4px;
        font-size: 13px;
        font-weight: 600;
        color: var(--ink);
        text-align: left;
    }

    .sec-h :global(svg),
    .chev-icon,
    .chev {
        color: var(--ink-3);
    }

    /* No hover or current-page boxes: colour alone shows them. */
    a:hover,
    .group:hover,
    .chev:hover,
    .sec-h:hover :global(svg) {
        color: var(--ink);
    }

    /* Child pages hang off a 1 px grey line. */
    ul ul {
        margin: 0 0 var(--s-2) 1px;
        padding-left: var(--s-4);
        box-shadow: inset 1px 0 0 var(--line);
    }

    a,
    .group {
        display: flex;
        align-items: center;
        gap: var(--s-2);
        width: 100%;
        min-height: 30px;
        padding: 5px 0;
        font-size: 14px;
        line-height: 20px;
        color: var(--ink-2);
        text-align: left;
        transition: color var(--dur-1) var(--ease);
    }

    .group {
        font-weight: 400;
    }

    .chev-icon {
        margin-left: auto;
        display: inline-flex;
    }

    .row {
        display: flex;
        align-items: center;
        gap: 2px;
    }

    .row a {
        flex: 1;
        min-width: 0;
    }

    .chev {
        flex: none;
        width: 28px;
        height: 28px;
        display: grid;
        place-items: center;
        margin-right: -8px;
        border-radius: var(--r-sm);
    }

    a[aria-current="page"] {
        color: var(--accent);
        font-weight: 600;
    }

    .sheet a,
    .sheet .group,
    .sheet .sec-h {
        min-height: 40px;
        font-size: 15px;
    }

    .sheet .chev {
        width: 40px;
        height: 40px;
    }
</style>
