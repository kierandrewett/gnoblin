<script lang="ts">
    // Bingux: a complete desktop for Gnoblin, a separate project. Written for people who use the desktop.
    import BxCommand from "#lib/components/bingux/BxCommand.svelte";
    import BxGroup from "#lib/components/bingux/BxGroup.svelte";
    import BxRow from "#lib/components/bingux/BxRow.svelte";
    import Screenshot from "#lib/components/content/Screenshot.svelte";
    import { docsHref } from "#lib/nav.ts";
    import { BINGUX_REPO, SETUP_COMMAND } from "#lib/site.ts";

    const HALF = "(max-width: 733px) 100vw, 536px";
</script>

<svelte:head>
    <title>Bingux · a desktop for Gnoblin</title>
    <meta
        name="description"
        content="Bingux is a complete desktop for Gnoblin, with a top bar, a dock, search, notifications and a sidebar."
    />
</svelte:head>

{#snippet appIcon(name: string)}
    <img src="/bingux/{name}.svg" alt="" width="40" height="40" />
{/snippet}

<section class="bxw-hero">
    <div class="bxw">
        <div class="text">
            <h1>Bingux</h1>
            <p class="lede">
                A complete desktop for Gnoblin, with a top bar, a dock, search, notifications and a sidebar. Install it
                and log in.
            </p>
            <div class="bxw-col setup">
                <BxGroup>
                    <div class="brow cmd-row"><BxCommand command={SETUP_COMMAND} /></div>
                </BxGroup>
            </div>
            <p class="shot-caption note">
                When it asks, press Space to choose Bingux, then Enter. Then install the bingux package and log in to
                Gnoblin.
            </p>
        </div>
        <div class="hero-shot">
            <Screenshot
                name="gnoblin-bingux-desktop"
                scheme
                sizes="(max-width: 1120px) 100vw, 1072px"
                alt="The Bingux desktop with Files, Settings and a terminal open, the top bar above and the dock below"
                caption="The Bingux desktop."
            />
        </div>
    </div>
</section>

<section class="bxw-band tint" aria-labelledby="what">
    <div class="bxw band-motion">
        <h2 id="what">What you get</h2>
        <p class="intro">Everything you need to use your computer, ready when you log in.</p>
        <div class="bxw-col">
            <BxGroup>
                <BxRow title="Top bar" sub="The time, network and sound, and controls for whatever is playing" />
                <BxRow title="Dock" sub="Your favourite apps and the ones that are open" />
                <BxRow title="Search" sub="Find and open apps by typing their name" />
                <BxRow title="Notifications" sub="Messages from your apps, gathered in one place" />
                <BxRow title="Sidebar" sub="Notes, a terminal and information about your computer" />
            </BxGroup>
        </div>
        <div class="bxw-two">
            <Screenshot
                name="gnoblin-bingux-search"
                scheme
                sizes={HALF}
                alt="Bingux search open over the desktop, with results for files"
                caption="Search: type to find an app."
            />
            <Screenshot
                name="gnoblin-bingux-notification"
                scheme
                sizes={HALF}
                alt="A notification from Files in the top corner of the Bingux desktop"
                caption="Notifications: they appear in the corner and wait in the bar."
            />
        </div>
    </div>
</section>

<section class="bxw-band" aria-labelledby="apps">
    <div class="bxw band-motion">
        <h2 id="apps">Apps</h2>
        <p class="intro">Install the ones you want, then open them from search or the dock.</p>
        <div class="bxw-col">
            <BxGroup>
                <BxRow title="Settings" sub="Wi-Fi, sound, displays, appearance and more">
                    {#snippet leading()}{@render appIcon("settings")}{/snippet}
                </BxRow>
                <BxRow title="Files" sub="Browse, open and organise your files">
                    {#snippet leading()}{@render appIcon("files")}{/snippet}
                </BxRow>
                <BxRow title="Taxonomy" sub="A tour of how Bingux looks and behaves">
                    {#snippet leading()}{@render appIcon("taxonomy")}{/snippet}
                </BxRow>
            </BxGroup>
        </div>
        <div class="bxw-two">
            <Screenshot
                name="bingux-settings"
                scheme
                sizes={HALF}
                alt="Bingux Settings on the Appearance page"
                caption="Settings"
            />
            <Screenshot
                name="bingux-files"
                scheme
                sizes={HALF}
                alt="Bingux Files showing a folder of documents"
                caption="Files"
            />
        </div>
    </div>
</section>

<section class="bxw-band tint" aria-labelledby="more">
    <div class="bxw band-motion">
        <h2 id="more">Read more</h2>
        <p class="intro">Step-by-step help for installing Bingux and getting started.</p>
        <div class="bxw-col">
            <BxGroup>
                <BxRow
                    title="Install Bingux"
                    sub="Add the repositories, install the package and log in"
                    href={docsHref("bingux")}
                />
                <BxRow title="Bingux on GitHub" sub="Help, questions and the source" href={BINGUX_REPO} />
                <BxRow
                    title="Other desktops for Gnoblin"
                    sub="If you would rather put together your own"
                    href={docsHref("bring-your-own-shell")}
                />
            </BxGroup>
        </div>
    </div>
</section>

<style>
    .setup {
        margin-top: 28px;
    }

    .cmd-row {
        padding: 12px 16px;
    }

    .cmd-row > :global(*) {
        flex: 1;
    }

    .note {
        margin-top: 12px;
    }

    .hero-shot {
        margin-top: 56px;
    }
</style>
