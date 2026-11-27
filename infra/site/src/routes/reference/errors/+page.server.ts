import { errorsPage } from "#lib/server/reference-pages.ts";
import type { PageServerLoad } from "./$types";

export const load: PageServerLoad = () => {
    const page = errorsPage();
    return { page, crumbs: page.crumbs, outline: page.outline };
};
