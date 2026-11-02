import { error } from "@sveltejs/kit";
import { docsPages, loadDocsPage } from "#lib/server/docs.ts";
import type { EntryGenerator, PageServerLoad } from "./$types";

// Every page in docs/, prerendered at /docs/<path>.
export const entries: EntryGenerator = () => [...docsPages().keys()].map((slug) => ({ slug }));

export const load: PageServerLoad = async ({ params }) => {
    const page = await loadDocsPage(params.slug);
    if (!page) error(404, "Not found");
    return { page };
};
