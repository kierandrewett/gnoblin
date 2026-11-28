<script lang="ts">
    // The events of a class: the name a listener is given, the arguments the handler receives and what happened. Rust shows the generated
    // `on_<event>` method as declared.
    import type { EventRow } from "#lib/reference/model.ts";

    let { rows }: { rows: EventRow[] } = $props();
</script>

<!-- svelte-ignore a11y_no_noninteractive_tabindex -->
<div class="tbl-wrap" tabindex="0" role="region" aria-label="Events">
    <table class="tbl ref-tbl">
        <thead>
            <tr>
                <th scope="col" class="c-evt" data-only="typescript python">Event</th>
                <th scope="col" class="c-args" data-only="typescript python">Arguments</th>
                <th scope="col" class="c-decl" data-only="rust">Declaration</th>
                <th scope="col">Description</th>
            </tr>
        </thead>
        <tbody>
            {#each rows as row (row.id)}
                <tr id={row.id}>
                    <th scope="row" data-only="typescript python"
                        ><a class="anchor" href="#{row.id}">{@html row.nameHtml}</a></th
                    >
                    <td class="args" data-only="typescript python" data-label="Arguments">{@html row.argsHtml}</td>
                    <th scope="row" data-only="rust">{@html row.declHtml}</th>
                    <td data-label="Description">
                        {@html row.descHtml}
                        {#if row.moreHtml}<div class="more">{@html row.moreHtml}</div>{/if}
                    </td>
                </tr>
            {/each}
        </tbody>
    </table>
</div>
