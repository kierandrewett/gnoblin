<script lang="ts">
    // Errors while browsing (an unknown page reached by a client-side link). The static 404.html is /404.
    import { page } from "$app/state";
    import NotFound from "#lib/components/layout/NotFound.svelte";
</script>

<svelte:head><title>{page.status === 404 ? "Page not found" : "Error"} · Gnoblin</title></svelte:head>

{#if page.status === 404}
    <NotFound />
{:else}
    <div class="wrap error">
        <h1>Something went wrong</h1>
        <p>{page.error?.message}</p>
    </div>
{/if}

<style>
    .error {
        padding: var(--s-10) var(--page-pad);
        text-align: center;
    }

    h1 {
        font-size: var(--t-h1-size);
        line-height: var(--t-h1-lh);
        margin-bottom: var(--s-4);
    }
</style>
