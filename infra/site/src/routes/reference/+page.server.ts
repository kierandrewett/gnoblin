import { indexPage } from "#lib/server/reference-pages.ts";
import type { PageServerLoad } from "./$types";

export const load: PageServerLoad = () => {
    const page = indexPage();
    return {
        page,
        crumbs: [{ html: "Reference" }],
        outline: [
            ...page.packages.map((pkg) => ({
                id: pkg.id,
                label: pkg.name,
                count: pkg.areas.reduce((n, a) => n + a.count, 0),
            })),
            { id: "errors", label: "Errors", count: page.errors.codes.length },
        ],
    };
};
