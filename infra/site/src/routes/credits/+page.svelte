<script lang="ts">
    // Credits and licences for what the site ships from other projects.
    import { LICENCE, REPO } from "#lib/site.ts";
    import iconsCopying from "../../../static/icons.COPYING?raw";
</script>

<svelte:head>
    <title>Credits · Gnoblin</title>
    <meta name="description" content="Licences of the icons and marks used on the Gnoblin website." />
</svelte:head>

<div class="wrap credits">
    <h1>Credits</h1>
    <p class="lede">
        The Gnoblin website and its documentation are part of <a class="link" href={REPO}>Gnoblin</a>, free software
        under <a class="link" href={LICENCE.href}>{LICENCE.label}</a>. It uses the following from other projects.
    </p>

    <h2>Interface icons</h2>
    <p>
        The icons come from the Bingux icon set, whose shapes are derived from
        <a class="link" href="https://phosphoricons.com">Phosphor Icons</a>. They are used under the MIT licence:
    </p>
    <pre class="licence">{iconsCopying.trim()}</pre>
    <p class="small">
        The same text is at <a class="link" href="/icons.COPYING">/icons.COPYING</a>, next to the icons.
    </p>

    <h2>System and GitHub marks</h2>
    <p>
        The marks of Fedora, openSUSE, Debian, Arch Linux, Alpine Linux, NixOS and GitHub are drawn from
        <a class="link" href="https://simpleicons.org">Simple Icons</a>, released under
        <a class="link" href="https://creativecommons.org/publicdomain/zero/1.0/">CC0 1.0</a>. The logos themselves are
        trademarks of their owners. They are shown only to link to the install guide for each system and do not mean
        that the owners endorse Gnoblin.
    </p>

    <h2>Screenshots</h2>
    <p>
        The screenshots are real captures of Gnoblin running the Bingux desktop. Bingux is a separate project with its
        own licence.
    </p>

    <h2>Fonts</h2>
    <p>The site uses the fonts already on your device and downloads none.</p>
</div>

<style>
    .credits {
        max-width: calc(var(--measure) + 2 * var(--page-pad));
        padding-top: var(--s-9);
        padding-bottom: var(--s-9);
        color: var(--ink-2);
    }

    h1 {
        font-size: var(--t-h1-size);
        line-height: var(--t-h1-lh);
        font-weight: 700;
        color: var(--ink);
    }

    .lede {
        font-size: var(--t-lead-size);
        line-height: var(--t-lead-lh);
        color: var(--ink-3);
        margin: var(--s-4) 0 var(--s-6);
    }

    h2 + p {
        margin-top: 0;
    }

    h2 {
        font-size: var(--t-h2-size);
        line-height: var(--t-h2-lh);
        font-weight: 600;
        color: var(--ink);
        margin: var(--s-7) 0 var(--s-3);
    }

    p,
    .licence {
        margin-top: var(--s-4);
    }

    .licence {
        padding: var(--s-5);
        border-radius: var(--r-md);
        background: var(--code-bg);
        box-shadow: inset 0 0 0 1px var(--code-line);
        color: var(--code-ink);
        font-family: var(--font-mono);
        font-size: 13px;
        line-height: 20px;
        white-space: pre-wrap;
    }

    .small {
        font-size: var(--t-small-size);
        color: var(--ink-3);
    }

    @media (max-width: 733px) {
        .credits {
            padding-top: var(--s-7);
        }
    }
</style>
