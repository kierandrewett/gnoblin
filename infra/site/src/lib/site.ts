// Site-wide data: project links, the setup command, the systems with an install guide and the build information.
import { docsHref } from "./nav.ts";

export const REPO = "https://github.com/kierandrewett/gnoblin";
export const BINGUX_REPO = "https://github.com/kierandrewett/bingux";
export const SETUP_COMMAND = "curl -fsSL https://repo.gnoblin.org/setup.sh | sh";
export const SETUP_SCRIPT = "https://repo.gnoblin.org/setup.sh";

export const LICENCE = { label: "GPL-2.0-or-later", href: `${REPO}/blob/main/COPYING` };

/** Source, Issues and Releases: in the header and footer of every page. */
export const PROJECT_LINKS = [
    { label: "Source", href: REPO, icon: "github" },
    { label: "Issues", href: `${REPO}/issues` },
    { label: "Releases", href: `${REPO}/releases` },
];

export type SiteArea = "docs" | "install" | "configure" | "shells" | "develop";

export const SITE_NAV: { label: string; href: string; area: SiteArea }[] = [
    { label: "Docs", href: docsHref(""), area: "docs" },
    { label: "Install", href: "/install", area: "install" },
    { label: "Configure", href: docsHref("config"), area: "configure" },
    { label: "Shells", href: docsHref("bring-your-own-shell"), area: "shells" },
    { label: "Develop", href: docsHref("shell-integration"), area: "develop" },
];

export interface System {
    /** Simple Icons glyph in the sprite, or "" for the generic code icon. */
    glyph: string;
    name: string;
    /** Package manager line on the install page. */
    pm: string;
    /** Short name in the home page's row of links. */
    short: string;
    href: string;
}

export const SYSTEMS: System[] = [
    { glyph: "fedora", name: "Fedora", short: "Fedora", pm: "dnf", href: docsHref("install-fedora") },
    { glyph: "opensuse", name: "openSUSE", short: "openSUSE", pm: "zypper", href: docsHref("install-opensuse") },
    { glyph: "debian", name: "Debian", short: "Debian", pm: "apt", href: docsHref("install-debian") },
    {
        glyph: "archlinux",
        name: "Arch Linux",
        short: "Arch Linux",
        pm: "pacman, also CachyOS",
        href: docsHref("install-arch"),
    },
    { glyph: "alpinelinux", name: "Alpine", short: "Alpine", pm: "apk", href: docsHref("install-alpine") },
    { glyph: "nixos", name: "NixOS", short: "NixOS", pm: "Nix flake", href: docsHref("install-nixos") },
    {
        glyph: "",
        name: "Build from source",
        short: "From source",
        pm: "Any other system",
        href: docsHref("install-source"),
    },
];

export const BUILD = {
    sha: __BUILD_SHA__,
    short: __BUILD_SHA__.slice(0, 7),
    time: __BUILT_AT__,
    tree: `${REPO}/tree/${__BUILD_SHA__}`,
};
