import { error } from "@sveltejs/kit";
import { typesEntries, typesPage } from "#lib/server/reference-pages.ts";
import type { EntryGenerator, PageServerLoad } from "./$types";

export const entries: EntryGenerator = () => typesEntries();

export const load: PageServerLoad = async ({ params }) => {
    const found = await typesPage(params.package, params.area);
    if (!found) error(404, "Not found");
    return { page: found.page, crumbs: found.page.crumbs, outline: found.page.outline };
};
