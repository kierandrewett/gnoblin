import { referenceNav } from "#lib/server/reference-pages.ts";
import type { LayoutServerLoad } from "./$types";

// The tree of the reference, the same on every page of it.
export const load: LayoutServerLoad = () => ({ nav: referenceNav() });
