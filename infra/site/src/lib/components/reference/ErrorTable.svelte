<script lang="ts">
    // The error codes, what each means, and the members that raise it.
    import type { ErrorRow } from "#lib/reference/model.ts";

    let {
        rows,
        by = true,
        anchors = false,
        label = "Errors",
    }: { rows: ErrorRow[]; by?: boolean; anchors?: boolean; label?: string } = $props();
</script>

<!-- svelte-ignore a11y_no_noninteractive_tabindex -->
<div class="tbl-wrap" tabindex="0" role="region" aria-label={label}>
    <table class="tbl ref-tbl">
        <thead>
            <tr>
                <th scope="col" class="c-code">Code</th>
                <th scope="col">Raised when</th>
                {#if by}<th scope="col">Raised by</th>{/if}
            </tr>
        </thead>
        <tbody>
            {#each rows as row (row.code)}
                <tr id={anchors ? row.code : undefined}>
                    <th scope="row"><b class="nm">{row.code}</b></th>
                    <td data-label="Raised when">{@html row.meaning}</td>
                    {#if by}
                        <td data-label="Raised by" class="by">
                            {#each row.by as member, i (i)}{#if i > 0},
                                {/if}<span>{@html member}</span>{/each}{#if row.more > 0}
                                <span class="mute">and {row.more} more</span>{/if}
                        </td>
                    {/if}
                </tr>
            {/each}
        </tbody>
    </table>
</div>
