import { error } from "@sveltejs/kit";
import { classEntries, classPage, findClass } from "#lib/server/reference-pages.ts";
import type { EntryGenerator, PageServerLoad } from "./$types";

export const entries: EntryGenerator = () => classEntries();

export const load: PageServerLoad = async ({ params }) => {
    const found = findClass(params.package, params.name);
    if (!found) error(404, "Not found");
    const page = await classPage(found.pkg, found.c);
    return { page, crumbs: page.crumbs, outline: page.outline };
};
