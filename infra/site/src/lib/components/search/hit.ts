/** One search result: a page, or a section (heading) inside a page. */
export interface Hit {
    kind: "page" | "section";
    title: string;
    /** Breadcrumbs of a page, or the page a section is on. */
    path: string;
    url: string;
}
