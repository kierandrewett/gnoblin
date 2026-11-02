<script lang="ts">
    import { docsHref } from "#lib/nav.ts";
    import Icon from "../ui/Icon.svelte";

    let { trail, title }: { trail: { label: string; slug?: string }[]; title: string } = $props();
</script>

{#if trail.length}
    <nav class="crumbs" aria-label="Breadcrumb">
        <ol role="list">
            {#each trail as step (step.label)}
                <li>
                    {#if step.slug !== undefined}<a href={docsHref(step.slug)}>{step.label}</a>{:else}<span
                            >{step.label}</span
                        >{/if}
                    <Icon name="chevron-right" size={12} />
                </li>
            {/each}
            <li><span aria-current="page">{title}</span></li>
        </ol>
    </nav>
{/if}

<style>
    ol {
        display: flex;
        flex-wrap: wrap;
        align-items: center;
        gap: 4px;
        font-size: 13px;
        line-height: 20px;
        color: var(--ink-3);
        margin-bottom: var(--s-4);
    }

    li {
        display: inline-flex;
        align-items: center;
        gap: 4px;
    }

    a:hover {
        color: var(--ink);
        text-decoration: underline;
        text-underline-offset: 3px;
    }

    [aria-current] {
        overflow-wrap: anywhere;
    }
</style>
