<script lang="ts">
    // Install: one command, then a guide per system, then what to do after installing.
    import Command from "#lib/components/content/Command.svelte";
    import Kbd from "#lib/components/content/Kbd.svelte";
    import Steps from "#lib/components/install/Steps.svelte";
    import SystemCard from "#lib/components/install/SystemCard.svelte";
    import Band from "#lib/components/layout/Band.svelte";
    import MoreLink from "#lib/components/ui/MoreLink.svelte";
    import { docsHref } from "#lib/nav.ts";
    import { SETUP_COMMAND, SETUP_SCRIPT, SYSTEMS } from "#lib/site.ts";
</script>

<svelte:head>
    <title>Install Gnoblin</title>
    <meta name="description" content="Add the Gnoblin package repository, install Gnoblin and a shell, and log in." />
</svelte:head>

<section class="intro">
    <div class="wrap">
        <div class="text">
            <h1>Install Gnoblin</h1>
            <p class="what">
                Gnoblin is the window manager. A shell provides the bar, dock and launcher, so you install both.
            </p>
        </div>
        <div class="setup">
            <h2 class="label">Quick setup</h2>
            <Command command={SETUP_COMMAND} />
            <div class="notes">
                <p>
                    <b>Adds the repository.</b> The script sets up the Gnoblin repository and signing key for your
                    system. It installs no packages. <a class="link" href={SETUP_SCRIPT}>Read the script</a>
                </p>
                <p>
                    <b>Asks what you want.</b> Gnoblin is selected. Press <Kbd key="Space" /> to select Bingux too, then press
                    <Kbd key="Enter" />.
                </p>
            </div>
        </div>
    </div>
</section>

<Band tint label="Guides by system">
    <h2>Guides by system</h2>
    <p class="lede">Each guide installs, updates and removes Gnoblin with your package manager, step by step.</p>
    <div class="grid">
        {#each SYSTEMS as system (system.href)}
            <SystemCard href={system.href} glyph={system.glyph || "code"} name={system.name} detail={system.pm} />
        {/each}
        <SystemCard
            href={docsHref("platform-support")}
            glyph="monitor"
            name="Platform support"
            detail="Releases and architectures"
            action="Details"
            quiet
        />
    </div>
</Band>

<Band label="After installing">
    <h2 class="after">After installing</h2>
    <Steps steps={[shell, login, configure]} />
    <div class="more">
        <MoreLink href={docsHref("optional-packages")}>Optional packages</MoreLink>
        <MoreLink href={docsHref("package-repositories")}>Package repositories and signing key</MoreLink>
    </div>
</Band>

{#snippet shell()}
    <h3>Install a shell</h3>
    <p>Bingux, Waybar with Fuzzel and Mako, or your own layer-shell clients.</p>
    <MoreLink href={docsHref("bring-your-own-shell")}>Choose a shell</MoreLink>
{/snippet}

{#snippet login()}
    <h3>Choose Gnoblin at login</h3>
    <p>Log out. Choose your user, open the session menu and choose Gnoblin.</p>
    <MoreLink href={docsHref("troubleshooting")}>Troubleshooting</MoreLink>
{/snippet}

{#snippet configure()}
    <h3>Configure Gnoblin</h3>
    <p>Create <code class="inline">~/.config/gnoblin/init.lua</code> and start from the starter config.</p>
    <MoreLink href={docsHref("recipes/complete-starter-config")}>Starter config</MoreLink>
{/snippet}

<style>
    .intro {
        padding: var(--s-10) 0;
    }

    .text,
    .setup {
        max-width: 760px;
    }

    h1 {
        font-size: var(--t-h1-size);
        line-height: var(--t-h1-lh);
        font-weight: 700;
    }

    .what {
        font-size: var(--t-intro-size);
        line-height: var(--t-intro-lh);
        color: var(--ink-2);
        margin-top: var(--s-3);
        max-width: 40ch;
        text-wrap: balance;
    }

    .setup {
        margin-top: var(--s-7);
    }

    .label {
        font-weight: 400;
        margin-bottom: var(--s-3);
    }

    .notes {
        display: grid;
        grid-template-columns: 1fr 1fr;
        gap: var(--s-6);
        margin-top: var(--s-6);
        color: var(--ink-3);
    }

    .notes b {
        color: var(--ink);
        font-weight: 600;
    }

    h2:not(.label) {
        font-size: var(--t-h2-size);
        line-height: var(--t-h2-lh);
        font-weight: 600;
    }

    .lede {
        color: var(--ink-3);
        margin: var(--s-2) 0 var(--s-6);
        max-width: 62ch;
    }

    .grid {
        display: grid;
        grid-template-columns: repeat(4, 1fr);
        gap: 20px;
    }

    .after {
        margin-bottom: var(--s-6);
    }

    .more {
        display: flex;
        flex-wrap: wrap;
        gap: var(--s-4) var(--s-7);
        margin-top: var(--s-8);
    }

    @media (max-width: 1023px) {
        .grid {
            grid-template-columns: repeat(2, 1fr);
        }
    }

    @media (max-width: 733px) {
        .intro {
            padding: var(--s-8) 0;
        }

        .notes {
            grid-template-columns: 1fr;
            gap: var(--s-4);
        }

        .grid {
            gap: 12px;
        }
    }
</style>
