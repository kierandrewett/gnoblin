<script lang="ts">
    // The tree of the reference: Overview, Errors, then each package with its areas and their classes and types page. The area of the current
    // page is open. The menu sheet shows the same tree (`sheet`).
    import type { ReferenceNav } from "#lib/reference/model.ts";
    import Icon from "../ui/Icon.svelte";

    let { nav, current, sheet = false }: { nav: ReferenceNav; current: string; sheet?: boolean } = $props();

    // The package and area that hold the current page. Derived, so the prerendered page already shows them open.
    const here = $derived.by(() => {
        for (const pkg of nav.packages) {
            for (const area of pkg.areas) {
                if (area.classes.some((c) => c.href === current) || area.types === current)
                    return `${pkg.id}/${area.id}`;
            }
        }
        return "";
    });
    let opened = $state(new Set<string>());
    let closed = $state(new Set<string>());
    const isOpen = (id: string) => opened.has(id) || ((here === id || here.startsWith(`${id}/`)) && !closed.has(id));

    function toggle(id: string) {
        const show = !isOpen(id);
        opened = new Set(show ? [...opened, id] : [...opened].filter((k) => k !== id));
        closed = new Set(show ? [...closed].filter((k) => k !== id) : [...closed, id]);
    }
</script>

<nav class={["ref-tree", sheet && "sheet"]} aria-label="API reference">
    <div class="sec">
        <p class="sec-h">API reference</p>
        <ul role="list">
            <li><a href="/reference" aria-current={current === "/reference" ? "page" : undefined}>Overview</a></li>
            <li>
                <a href="/reference/errors" aria-current={current === "/reference/errors" ? "page" : undefined}
                    >Errors</a
                >
            </li>
        </ul>
    </div>
    {#each nav.packages as pkg (pkg.id)}
        {@const open = isOpen(pkg.id)}
        <div class="sec">
            <button type="button" class="sec-h mono" aria-expanded={open} onclick={() => toggle(pkg.id)}>
                {pkg.name}
                <Icon name={open ? "chevron-down" : "chevron-right"} size={12} />
            </button>
            {#if open}
                <ul role="list">
                    {#each pkg.areas as area (area.id)}
                        {@const id = `${pkg.id}/${area.id}`}
                        {@const areaOpen = isOpen(id)}
                        <li>
                            <button type="button" class="grp" aria-expanded={areaOpen} onclick={() => toggle(id)}>
                                {area.title}
                                <Icon name={areaOpen ? "chevron-down" : "chevron-right"} size={12} />
                            </button>
                            {#if areaOpen}
                                <ul role="list">
                                    {#each area.classes as c (c.href)}
                                        <li>
                                            <a href={c.href} aria-current={c.href === current ? "page" : undefined}
                                                >{@html c.html}</a
                                            >
                                        </li>
                                    {/each}
                                    {#if area.types}
                                        <li>
                                            <a
                                                href={area.types}
                                                aria-current={area.types === current ? "page" : undefined}>Types</a
                                            >
                                        </li>
                                    {/if}
                                </ul>
                            {/if}
                        </li>
                    {/each}
                </ul>
            {/if}
        </div>
    {/each}
</nav>
