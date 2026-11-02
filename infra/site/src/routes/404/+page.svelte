<script lang="ts">
    // Prerendered as build/404.html, which Cloudflare (not_found_handling = "404-page") serves for unknown URLs.
    import NotFound from "#lib/components/layout/NotFound.svelte";
</script>

<svelte:head>
    <title>Page not found · Gnoblin</title>
    <meta name="robots" content="noindex" />
</svelte:head>

<NotFound />
