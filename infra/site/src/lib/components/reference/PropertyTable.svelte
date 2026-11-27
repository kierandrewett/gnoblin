<script lang="ts">
    // The properties of a class: Name, Type, Flags and Description. Rust has no type to read in a row: its first column is the declaration, the
    // getter and the setters as the generated API has them (which language shows which cell is `data-only`, see styles/reference.css).
    import type { PropertyRow } from "#lib/reference/model.ts";

    let { rows, label }: { rows: PropertyRow[]; label: string } = $props();
</script>

<!-- svelte-ignore a11y_no_noninteractive_tabindex -->
<div class="tbl-wrap" tabindex="0" role="region" aria-label={label}>
    <table class="tbl ref-tbl">
        <thead>
            <tr>
                <th scope="col" class="c-name" data-only="typescript python">Name</th>
                <th scope="col" class="c-type" data-only="typescript python">Type</th>
                <th scope="col" class="c-decl" data-only="rust">Declaration</th>
                <th scope="col" class="c-flags">Flags</th>
                <th scope="col">Description</th>
            </tr>
        </thead>
        <tbody>
            {#each rows as row (row.id)}
                <tr id={row.id}>
                    <th scope="row" data-only="typescript python"
                        ><a class="anchor" href="#{row.id}">{@html row.nameHtml}</a></th
                    >
                    <td class="t" data-only="typescript python" data-label="Type">{@html row.typeHtml}</td>
                    <th scope="row" data-only="rust">{@html row.declHtml}</th>
                    <td class="flags" data-label="Flags">
                        {row.access}{#if row.animatable}<i>animatable</i>{/if}
                    </td>
                    <td data-label="Description">
                        {@html row.descHtml}
                        {#each row.facts as fact, i (i)}<span class="df">{@html fact}</span>{/each}
                        {#if row.moreHtml}<div class="more">{@html row.moreHtml}</div>{/if}
                    </td>
                </tr>
            {/each}
        </tbody>
    </table>
</div>
