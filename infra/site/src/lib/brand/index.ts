// The Gnoblin brand files: the window goblin from ~/dev/gnoblin-icon (dist/gnoblin-mark.svg, gnoblin-logo.svg and
// gnoblin-favicon.svg). The mark draws in currentColor, so it takes the text colour. The favicon switches colour under
// prefers-color-scheme. The site shows the mark next to the name as text, so logo.svg (the full wordmark) is kept here
// for use outside the pages.
import faviconUrl from "./favicon.svg?url";
import markSource from "./mark.svg?raw";

/** The mark's SVG source. It draws in currentColor. */
export const MARK_SVG = markSource;
export const FAVICON_URL = faviconUrl;
