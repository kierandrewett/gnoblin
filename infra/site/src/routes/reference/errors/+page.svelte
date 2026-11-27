<script lang="ts">
    // Every error code, what it means and the members that raise it.
    import ErrorTable from "#lib/components/reference/ErrorTable.svelte";
    import type { PageProps } from "./$types";

    let { data }: PageProps = $props();
    const e = $derived(data.page);
</script>

<svelte:head>
    <title>Errors · API reference · Gnoblin</title>
    <meta
        name="description"
        content="The error codes of the Gnoblin API: what each means and which members raise it."
    />
</svelte:head>

<h1>Errors</h1>
<p class="ref-lede">One set of codes covers every failure of the API, the same in TypeScript, Python and Rust.</p>

<h2 id="codes">Codes <span class="cnt">{e.rows.length}</span></h2>
<ErrorTable rows={e.rows} anchors label="Error codes" />

<h2 id="failures">Failures</h2>
<div class="ref-prose">
    {#each Object.entries(e.prose) as [lang, html] (lang)}<p data-only={lang}>{@html html}</p>{/each}
</div>
