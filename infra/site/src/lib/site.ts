// Site-wide data: project links, the setup command, the systems with an install guide and the build information.
import { docsHref, NAV, type NavItem, type NavSection } from "./nav.ts";

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

/** The header's areas are the sidebar's sections (the same ids and words), plus the reference. */
export type SiteArea = NavSection["id"] | "reference";

/** The first page of a section, in sidebar order: where its header link goes. */
function firstSlug(items: NavItem[]): string | undefined {
    for (const item of items) {
        if (item.slug !== undefined) return item.slug;
        const inner = item.items && firstSlug(item.items);
        if (inner !== undefined) return inner;
    }
    return undefined;
}

export const SITE_NAV: { label: string; href: string; area: SiteArea }[] = [
    ...NAV.map((section) => ({
        label: section.label,
        href: docsHref(firstSlug(section.items) ?? ""),
        area: section.id,
    })),
    { label: "Reference", href: docsHref("reference"), area: "reference" },
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
