// Site-wide data: project links, the setup command, the systems with an install guide and the build information.
import { docsHref, NAV, type NavItem, type NavSection } from "./nav.ts";

export const REPO = "https://github.com/kierandrewett/gnoblin";
export const BINGUX_REPO = "https://github.com/kierandrewett/bingux";
export const SETUP_COMMAND = "curl -fsSL https://repo.gnoblin.org/setup.sh | sh";

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

export const BUILD = {
    sha: __BUILD_SHA__,
    short: __BUILD_SHA__.slice(0, 7),
    time: __BUILT_AT__,
    tree: `${REPO}/tree/${__BUILD_SHA__}`,
};
