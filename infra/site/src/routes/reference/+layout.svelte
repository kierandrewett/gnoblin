<script lang="ts">
    // The reference pages' shell: the tree at the left, the article in the middle with a bar of breadcrumbs and the language picker, and the
    // outline at the right. The outline moves into the article below 1280 px and the tree into the menu sheet below 1024 px (the root layout
    // gives the sheet the same tree). Each page brings its breadcrumbs and its outline in its data.
    import "#lib/styles/reference.css";
    import { onMount, type Snippet } from "svelte";
    import { page } from "$app/state";
    import Details from "#lib/components/content/Details.svelte";
    import LanguagePicker from "#lib/components/reference/LanguagePicker.svelte";
    import Outline from "#lib/components/reference/Outline.svelte";
    import ReferenceTree from "#lib/components/reference/ReferenceTree.svelte";
    import type { Crumb, OutlineItem, ReferenceNav } from "#lib/reference/model.ts";

    let { data, children }: { data: { nav: ReferenceNav }; children: Snippet } = $props();

    const crumbs = $derived((page.data.crumbs as Crumb[] | undefined) ?? []);
    const outline = $derived((page.data.outline as OutlineItem[] | undefined) ?? []);

    // The bar gains a rule once the page has scrolled under it.
    let sentinel: HTMLElement;
    let stuck = $state(false);
    onMount(() => {
        const observer = new IntersectionObserver(([entry]) => (stuck = !entry!.isIntersecting), {
            rootMargin: "-1px 0px 0px 0px",
        });
        observer.observe(sentinel);
        return () => observer.disconnect();
    });
</script>

<div class="ref">
    <div class="ref-navcol">
        <div class="ref-sticky"><ReferenceTree nav={data.nav} current={page.url.pathname} /></div>
    </div>
    <div class="ref-art">
        <div bind:this={sentinel} aria-hidden="true"></div>
        <div class={["ref-bar", stuck && "stuck"]}>
            <nav class="ref-crumbs" aria-label="Breadcrumb">
                <ol>
                    {#each crumbs as crumb, i (i)}
                        <li aria-current={i === crumbs.length - 1 ? "page" : undefined}>
                            {#if crumb.href && i < crumbs.length - 1}<a href={crumb.href}>{@html crumb.html}</a
                                >{:else}<span>{@html crumb.html}</span>{/if}
                        </li>
                    {/each}
                </ol>
            </nav>
            <LanguagePicker />
        </div>
        <article>
            {@render children()}
            {#if outline.length}
                <div class="ref-mtoc">
                    <Details summary="On this page">
                        <ul role="list">
                            {#each outline as item (item.id)}<li><a href="#{item.id}">{item.label}</a></li>{/each}
                        </ul>
                    </Details>
                </div>
            {/if}
        </article>
    </div>
    <div class="ref-toccol">
        <div class="ref-sticky"><Outline items={outline} /></div>
    </div>
</div>
