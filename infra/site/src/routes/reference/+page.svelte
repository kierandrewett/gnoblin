<script lang="ts">
    // The index: every area of each package with its classes, records, enums and the rest, as links. The header search filters it in place
    // on this page (the index has no filter row of its own).
    import ImportLine from "#lib/components/reference/ImportLine.svelte";
    import type { PageProps } from "./$types";

    let { data }: PageProps = $props();
</script>

<svelte:head>
    <title>API reference · Gnoblin</title>
    <meta
        name="description"
        content="Every class, record and enum of gnoblin-core and gnoblin-std, with its properties, methods and events in TypeScript, Python and Rust."
    />
</svelte:head>

<h1>API reference</h1>
<p class="ref-lede">
    Every class, record and enum of <code>gnoblin-core</code> and <code>gnoblin-std</code>, with its properties, methods
    and events in TypeScript, Python and Rust. A name is spelled the way the language picked above spells it.
</p>

{#each data.page.packages as pkg (pkg.id)}
    <section class="ref-pkg" id={pkg.id} aria-labelledby="{pkg.id}-h">
        <h2 id="{pkg.id}-h">{pkg.name}</h2>
        <p class="ref-note">{pkg.blurb}</p>
        <ImportLine text={pkg.importText} />
        {#each pkg.areas as area (area.id)}
            <section class="ref-area" id="{pkg.id}-{area.id}" aria-labelledby="{pkg.id}-{area.id}-h">
                <h3 id="{pkg.id}-{area.id}-h">{area.title}<span class="cnt">{area.count}</span></h3>
                <dl class="ref-kinds">
                    {#each area.groups as group (group.label)}
                        <dt>{group.label}</dt>
                        <dd>
                            {#each group.links as link (link.href)}<a href={link.href}>{@html link.html}</a>{/each}
                        </dd>
                    {/each}
                </dl>
            </section>
        {/each}
    </section>
{/each}

<section class="ref-pkg" id="errors" aria-labelledby="errors-h">
    <h2 id="errors-h">Errors</h2>
    <p class="ref-note">
        A call that cannot be made fails with one of these codes, the same set in every language. <a
            class="ty"
            href="/reference/errors">Errors</a
        >
        says what each means and which members raise it.
    </p>
    <dl class="ref-kinds">
        <dt>Codes</dt>
        <dd>
            {#each data.page.errors.codes as c (c.code)}<a href="/reference/errors#{c.code}">{c.code}</a>{/each}
        </dd>
    </dl>
</section>
