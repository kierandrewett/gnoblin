<script lang="ts">
    // Home: one centred 720 px document column in the restrained style of mutter.gnome.org. Links in the accent
    // colour; the layout's header bar and one-line home footer frame it.
    import Screenshot from "#lib/components/content/Screenshot.svelte";
    import Mark from "#lib/components/ui/Mark.svelte";
    import { docsHref } from "#lib/nav.ts";
    import { PROJECT_LINKS } from "#lib/site.ts";

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
    <p class="cta">
        <a class="btn primary" href="/install">Install</a>
        <a class="btn secondary" href={docsHref("")}>Read the docs</a>
    </p>

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
            <div class="card">
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

    .cta {
        display: flex;
        flex-wrap: wrap;
        gap: var(--s-3);
        margin: var(--s-5) 0 0;
    }

    .doc a.btn {
        display: inline-flex;
        align-items: center;
        height: 44px;
        padding: 0 var(--s-5);
        border-radius: var(--r-pill);
        border: 1px solid var(--accent);
        font-weight: 600;
        text-decoration: none;
        transition:
            background-color var(--dur-1) var(--ease),
            border-color var(--dur-1) var(--ease);
    }

    .doc a.btn.primary {
        background: var(--accent);
        color: var(--on-accent);
    }

    .doc a.btn.primary:hover {
        background: var(--accent-strong);
        border-color: var(--accent-strong);
        text-decoration: none;
    }

    .doc a.btn.secondary {
        color: var(--accent);
    }

    .doc a.btn.secondary:hover {
        background: var(--accent-soft);
        text-decoration: none;
    }

    .pairs {
        display: grid;
        grid-template-columns: 1fr 1fr;
        gap: var(--s-4);
    }

    .card {
        position: relative;
        padding: var(--s-4);
        border: 1px solid var(--line);
        border-radius: var(--r-md);
        background: var(--surface);
        transition:
            transform 140ms var(--ease),
            border-color 140ms var(--ease),
            box-shadow 140ms var(--ease);
    }

    .card:hover,
    .card:focus-within {
        transform: translateY(-2px);
        border-color: var(--accent);
        box-shadow: 0 8px 24px -8px rgba(var(--shadow-rgb), 0.25);
    }

    .card h3 a::after {
        content: "";
        position: absolute;
        inset: 0;
        border-radius: var(--r-md);
    }

    .card p {
        margin: 0;
        color: var(--ink-2);
    }

    @media (prefers-reduced-motion: reduce) {
        .card,
        .doc a.btn {
            transition: none;
        }

        .card:hover,
        .card:focus-within {
            transform: none;
        }
    }

    @media (max-width: 733px) {
        .doc {
            padding-top: var(--s-7);
        }

        .pairs {
            grid-template-columns: 1fr;
        }
    }
</style>
