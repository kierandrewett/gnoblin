// The brand slot. PLACEHOLDERS: the owner has not chosen a logo. Replace mark.svg, logo.svg and favicon.svg in this
// folder with the chosen concept (see README.md) and every page picks them up. The site shows the mark next to the
// name as text, so logo.svg (the full wordmark) is kept here for use outside the pages.
import faviconUrl from "./favicon.svg?url";
import markSource from "./mark.svg?raw";

/** The mark's SVG source. It draws in currentColor. */
export const MARK_SVG = markSource;
export const FAVICON_URL = faviconUrl;
