import { json } from "@sveltejs/kit";
import { searchRecords } from "#lib/server/reference-pages.ts";
import type { RequestHandler } from "./$types";

// Prerendered to build/reference/search-records.json, which scripts/postbuild.mjs adds to the search index and deletes. It is the one entry
// of the index for each class and each member of the reference: found by any of its three spellings, shown in the language that is picked.
export const prerender = true;

export const GET: RequestHandler = () => json(searchRecords());
