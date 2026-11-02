<script lang="ts">
    // A data table. Cells are snippets so they can hold inline code and links.
    import type { Snippet } from "svelte";

    let { head, rows }: { head: string[]; rows: Snippet[][] } = $props();
</script>

<div class="tbl-wrap">
    <table class="tbl">
        <thead>
            <tr>
                {#each head as cell, i (i)}<th scope="col">{cell}</th>{/each}
            </tr>
        </thead>
        <tbody>
            {#each rows as row, r (r)}
                <tr>
                    {#each row as cell, c (c)}<td>{@render cell()}</td>{/each}
                </tr>
            {/each}
        </tbody>
    </table>
</div>
