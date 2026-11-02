<script lang="ts">
    // The outline of the page (h2 and h3) with the heading in view marked, then edit and report links.
    import type { Heading } from "#lib/docs.ts";
    import { REPO } from "#lib/site.ts";
    import Icon from "../ui/Icon.svelte";

    let { headings, file }: { headings: Heading[]; file: string } = $props();

    let active = $state("");

    $effect(() => {
        const targets = headings
            .map((h) => document.getElementById(h.id))
            .filter((el): el is HTMLElement => el !== null);
        let frame = 0;
        const update = () => {
            frame = 0;
            // The last heading that has scrolled past the top bar; the first one before any has.
            const line = 96;
            let current = targets[0]?.id ?? "";
            for (const el of targets) {
                if (el.getBoundingClientRect().top <= line) current = el.id;
                else break;
            }
            if (window.innerHeight + window.scrollY >= document.documentElement.scrollHeight - 2) {
                current = targets.at(-1)?.id ?? current;
            }
            active = current;
        };
        const onScroll = () => {
            if (!frame) frame = requestAnimationFrame(update);
        };
        update();
        window.addEventListener("scroll", onScroll, { passive: true });
        return () => {
            window.removeEventListener("scroll", onScroll);
            cancelAnimationFrame(frame);
        };
    });
</script>

<aside class="toc" aria-label="On this page">
    {#if headings.length}
        <h2>On this page</h2>
        <ul role="list">
            {#each headings as h (h.id)}
                <li>
                    <a href="#{h.id}" class:sub={h.depth === 3} aria-current={h.id === active ? "location" : undefined}
                        >{h.text}</a
                    >
                </li>
            {/each}
        </ul>
    {/if}
    <div class="acts">
        <a href="{REPO}/edit/main/docs/{file}"><Icon name="edit" size={15} />Edit this page</a>
        <a href="{REPO}/issues/new"><Icon name="arrow-out" size={15} />Report a problem</a>
    </div>
</aside>

<style>
    .toc {
        position: sticky;
        top: var(--topbar-h);
        max-height: calc(100vh - var(--topbar-h));
        overflow-y: auto;
        padding: var(--s-7) var(--s-5) var(--s-6) 0;
    }

    h2 {
        font-size: 13px;
        line-height: 20px;
        font-weight: 600;
        margin-bottom: var(--s-3);
    }

    ul {
        display: flex;
        flex-direction: column;
        gap: 2px;
    }

    li a {
        display: block;
        padding: 4px 0;
        font-size: 13px;
        line-height: 18px;
        color: var(--ink-3);
        transition: color var(--dur-1) var(--ease);
    }

    li a:hover {
        color: var(--ink);
    }

    li a.sub {
        padding-left: var(--s-3);
    }

    li a[aria-current] {
        color: var(--ink);
        font-weight: 600;
    }

    .acts {
        margin-top: var(--s-5);
        padding-top: var(--s-4);
        border-top: 1px solid var(--line);
        display: flex;
        flex-direction: column;
        gap: var(--s-2);
    }

    .acts a {
        display: inline-flex;
        align-items: center;
        gap: 6px;
        font-size: 13px;
        color: var(--accent);
    }

    .acts a:hover {
        text-decoration: underline;
        text-underline-offset: 3px;
    }
</style>
