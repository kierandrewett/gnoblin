<script lang="ts">
    // "On this page": the sections of a reference page, with a count where a section lists members. The one in view is current.
    import { onMount } from "svelte";
    import type { OutlineItem } from "#lib/reference/model.ts";

    let { items, label = "On this page" }: { items: OutlineItem[]; label?: string } = $props();
    let active = $state("");

    onMount(() => {
        const sections = items
            .map((item) => document.getElementById(item.id))
            .filter((el): el is HTMLElement => el !== null);
        if (!sections.length || !("IntersectionObserver" in window)) return;
        const seen: string[] = [];
        const observer = new IntersectionObserver(
            (entries) => {
                for (const entry of entries) {
                    const at = seen.indexOf(entry.target.id);
                    if (entry.isIntersecting && at === -1) seen.push(entry.target.id);
                    else if (!entry.isIntersecting && at !== -1) seen.splice(at, 1);
                }
                active = items.find((item) => seen.includes(item.id))?.id ?? active;
            },
            { rootMargin: "-72px 0px -65% 0px" },
        );
        for (const section of sections) observer.observe(section);
        return () => observer.disconnect();
    });
</script>

<aside class="ref-toc" aria-label={label}>
    <p class="t">{label}</p>
    <ul>
        {#each items as item (item.id)}
            <li>
                <a href="#{item.id}" aria-current={active === item.id ? "true" : undefined}>
                    <span>{item.label}</span>
                    {#if item.count !== undefined}<span class="c">{item.count}</span>{/if}
                </a>
            </li>
        {/each}
    </ul>
</aside>
