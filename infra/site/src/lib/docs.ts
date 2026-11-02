// Types of a rendered docs page, shared by the build-time loader (src/lib/server) and the page components.
import type { NavEntry, NavSection } from "./nav.ts";

export interface Heading {
    depth: 2 | 3;
    id: string;
    text: string;
}

export interface DocsPage {
    slug: string;
    /** Path under docs/, for the edit link. */
    file: string;
    title: string;
    /** HTML of the paragraph right after the title. */
    lede: string;
    /** HTML of the rest of the page. */
    body: string;
    headings: Heading[];
    /** Breadcrumbs above the title, from the sidebar. */
    trail: NavEntry["trail"];
    section: NavSection["id"] | null;
    prev: { label: string; href: string } | null;
    next: { label: string; href: string } | null;
}
