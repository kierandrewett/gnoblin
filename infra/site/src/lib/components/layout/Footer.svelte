<script lang="ts">
    // One line: licence and project links, then the time and commit of this build. The home page has a centred
    // sentence instead, in the plain style of the rest of that page.
    import { BUILD, LICENCE, PROJECT_LINKS } from "#lib/site.ts";

    let { variant = "site" }: { variant?: "site" | "home" } = $props();
</script>

{#if variant === "home"}
    <footer class="home">
        Gnoblin is free software under <a href={LICENCE.href}>{LICENCE.label}</a>. Built
        <time datetime={BUILD.time}>{BUILD.time}</time> from <a href={BUILD.tree}>{BUILD.short}</a>.
        <a href="/credits" class="quiet">Credits</a>
    </footer>
{:else}
    <footer class="footer">
        <div class="wrap base">
            <nav class="links" aria-label="Project">
                <a href={LICENCE.href}>{LICENCE.label}</a>
                {#each PROJECT_LINKS as link (link.label)}<a href={link.href}>{link.label}</a>{/each}
                <a href="/credits" class="quiet">Credits</a>
            </nav>
            <p class="built">
                built <time datetime={BUILD.time}>{BUILD.time}</time> from <a href={BUILD.tree}>{BUILD.short}</a>
            </p>
        </div>
    </footer>
{/if}

<style>
    .home {
        width: min(720px, 100% - 40px);
        margin: 0 auto;
        text-align: center;
        font-size: 14px;
        line-height: 22px;
        color: var(--ink-3);
        padding: var(--s-9) 0 var(--s-8);
    }

    .home a {
        color: var(--accent);
        font-weight: 500;
        text-decoration: none;
    }

    .home a:hover {
        text-decoration: underline;
        text-underline-offset: 3px;
    }

    .home a.quiet {
        color: var(--ink-3);
        margin-left: var(--s-2);
    }

    .footer {
        background: var(--bg-alt);
        border-top: 1px solid var(--line);
        color: var(--ink-3);
        padding: var(--s-6) 0;
        font-family: var(--font-mono);
        font-size: 12px;
        line-height: 18px;
    }

    .base {
        display: flex;
        align-items: center;
        flex-wrap: wrap;
        gap: var(--s-2) var(--s-5);
    }

    .links {
        display: flex;
        flex-wrap: wrap;
        gap: var(--s-2) var(--s-5);
    }

    a {
        color: var(--ink-2);
        text-decoration: underline;
        text-underline-offset: 2px;
        text-decoration-color: var(--line-strong);
    }

    a:hover {
        color: var(--ink);
        text-decoration-color: currentColor;
    }

    a.quiet {
        color: var(--ink-3);
    }

    .built {
        margin-left: auto;
    }

    @media (max-width: 733px) {
        .built {
            margin-left: 0;
        }
    }
</style>
