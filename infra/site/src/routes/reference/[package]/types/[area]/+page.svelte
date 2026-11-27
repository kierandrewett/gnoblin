<script lang="ts">
    // The types of one area: its records, interfaces, enums and callbacks on one page, each with an anchor of its schema name.
    import type { PageProps } from "./$types";

    let { data }: PageProps = $props();
    const t = $derived(data.page);
</script>

<svelte:head>
    <title>Types of {t.area} · API reference · Gnoblin</title>
    <meta name="description" content="The records, enums, interfaces and callbacks of {t.area} in {t.package}." />
</svelte:head>

<h1>Types</h1>
<p class="ref-lede">The records, enums, interfaces and callbacks of <em>{t.area}</em> in <code>{t.package}</code>.</p>

{#each t.sections as section (section.id)}
    <section id={section.id} aria-labelledby="{section.id}-h">
        <h2 id="{section.id}-h" class="ident">
            <span>{@html section.nameHtml}</span><span class="cnt">{section.kind} · since {section.since}</span>
        </h2>
        <div class="ref-prose">{@html section.descHtml}</div>
        {@html section.bodyHtml}
    </section>
{/each}
