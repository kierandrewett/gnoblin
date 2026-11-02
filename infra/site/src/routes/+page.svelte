<script lang="ts">
    // Home: one centred 720 px document column in the restrained style of mutter.gnome.org. No header bar, cards,
    // boxes, bands or tables; links in the accent colour; a one-line footer (the layout's home footer).
    import Command from "#lib/components/content/Command.svelte";
    import Screenshot from "#lib/components/content/Screenshot.svelte";
    import DistroLinks from "#lib/components/install/DistroLinks.svelte";
    import Mark from "#lib/components/ui/Mark.svelte";
    import { docsHref } from "#lib/nav.ts";
    import { PROJECT_LINKS, SETUP_COMMAND } from "#lib/site.ts";

    const REFERENCE = [
        {
            title: "Window rules",
            href: docsHref("config/window_rule"),
            text: "Match windows by app or title and change how they look, where they open and how they behave.",
        },
        {
            title: "Animation",
            href: docsHref("config/animation"),
            text: "Name animations and use them when windows and panels open and close.",
        },
        {
            title: "Input",
            href: docsHref("config/configure/input"),
            text: "Keyboards, mice, touchpads, tablets and pens.",
        },
        { title: "Shortcuts", href: docsHref("config/configure/shortcuts"), text: "Bind any command to a key." },
        {
            title: "Monitors",
            href: docsHref("config/configure/monitors"),
            text: "Settings for the screens you connect.",
        },
        {
            title: "Portals",
            href: docsHref("config/configure/portals"),
            text: "Which programs answer apps that ask for files, screenshots and more.",
        },
    ];
</script>

<svelte:head>
    <title>Gnoblin</title>
    <meta
        name="description"
        content="Gnoblin runs the windows and apps on your Linux desktop. Choose your own desktop and set it up in one file."
    />
</svelte:head>

<div class="doc">
    <div class="top">
        <Mark height={72} />
        <h1>Gnoblin</h1>
    </div>

    <div class="shot">
        <Screenshot
            name="gnoblin-bingux-desktop"
            scheme
            alt="A Gnoblin session running the Bingux desktop, with Files, Settings and a terminal open"
        />
    </div>

    <p>
        Gnoblin runs the windows and apps on your Linux desktop. It places and focuses windows, handles your keyboard,
        mouse, touchpad and screens, and starts your desktop when you log in.
    </p>
    <p>
        What the desktop looks like is up to you. <a href="/bingux">Bingux</a> gives you a complete one, with a top bar, dock
        and search. You can also put one together from tools such as Waybar, Fuzzel and Mako, or write your own.
    </p>
    <p>
        You set Gnoblin up in one file, <code class="inline">~/.config/gnoblin/init.lua</code>, and changes apply
        without logging out.
    </p>

    <h2>Install</h2>
    <p>This adds the Gnoblin package repository for your system. Then install the package and a shell.</p>
    <Command command={SETUP_COMMAND} />
    <div class="distros"><DistroLinks /></div>

    <h2>Documentation</h2>
    <ul>
        <li><a href={docsHref("installation")}>Install</a></li>
        <li><a href={docsHref("bring-your-own-shell")}>Choose a shell</a></li>
        <li><a href={docsHref("config")}>Configure</a></li>
        <li><a href={docsHref("troubleshooting")}>Troubleshooting</a></li>
    </ul>

    <h2>Configuration reference</h2>
    <div class="pairs">
        {#each REFERENCE as item (item.href)}
            <div>
                <h3><a href={item.href}>{item.title}</a></h3>
                <p>{item.text}</p>
            </div>
        {/each}
    </div>

    <h2>Shell development</h2>
    <ul>
        <li><a href={docsHref("shell-integration")}>Integration guide</a>: what a shell needs to work with Gnoblin.</li>
        <li><a href={docsHref("bridge")}>Compositor bridge</a>: talk to Gnoblin from your own program.</li>
        <li><a href={docsHref("wayland-protocols")}>Wayland protocols</a>: the interfaces Gnoblin offers.</li>
    </ul>

    <h2>Useful links</h2>
    <ul>
        <li><a href={PROJECT_LINKS[0]!.href}>Source code</a></li>
        {#each PROJECT_LINKS.slice(1) as link (link.label)}<li><a href={link.href}>{link.label}</a></li>{/each}
        <li><a href="/bingux">Bingux</a></li>
    </ul>
</div>

<style>
    .doc {
        width: min(720px, 100% - 40px);
        margin: 0 auto;
        padding-top: var(--s-9);
        font-size: 16px;
        line-height: 1.6;
        color: var(--ink);
    }

    .doc a {
        color: var(--accent);
        font-weight: 600;
    }

    .doc a:hover {
        text-decoration: underline;
        text-underline-offset: 3px;
    }

    .top {
        display: flex;
        flex-direction: column;
        align-items: center;
        gap: var(--s-3);
    }

    h1 {
        font-size: var(--t-h1-size);
        line-height: var(--t-h1-lh);
        font-weight: 700;
    }

    .shot {
        margin: var(--s-8) 0 var(--s-7);
    }

    p {
        margin: 0 0 var(--s-4);
    }

    h2 {
        font-size: 1.25rem;
        line-height: 1.25;
        font-weight: 600;
        margin: var(--s-8) 0 var(--s-4);
    }

    h3 {
        font-size: 1.05rem;
        font-weight: 600;
        margin: 0 0 var(--s-2);
    }

    ul {
        margin: 0 0 var(--s-4) 22px;
        display: flex;
        flex-direction: column;
        gap: 2px;
    }

    .distros {
        margin-top: var(--s-4);
    }

    .pairs {
        display: grid;
        grid-template-columns: 1fr 1fr;
        gap: var(--s-6) var(--s-8);
        padding: var(--s-2) var(--s-4) 0;
    }

    .pairs p {
        margin: 0;
        color: var(--ink-2);
    }

    @media (max-width: 733px) {
        .doc {
            padding-top: var(--s-7);
        }

        .pairs {
            grid-template-columns: 1fr;
            padding: 0;
        }
    }
</style>
