<script lang="ts">
    import { REPO } from "#lib/site.ts";
    import Icon from "../ui/Icon.svelte";

    let { file }: { file: string } = $props();
</script>

<div class="pagefoot">
    <a href="{REPO}/edit/main/docs/{file}"><Icon name="edit" size={16} />Edit this page on GitHub</a>
</div>

<style>
    .pagefoot {
        margin-top: var(--s-8);
        padding-top: var(--s-5);
        border-top: 1px solid var(--line);
        display: flex;
        font-size: 14px;
    }

    a {
        display: inline-flex;
        align-items: center;
        gap: 6px;
        color: var(--accent);
    }

    a:hover {
        text-decoration: underline;
        text-underline-offset: 3px;
    }
</style>
