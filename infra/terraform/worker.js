// The website Worker (infra/terraform/main.tf). It serves the build in infra/site/build as static assets, and answers the old URLs with a real 301.
//
// A request for a file of the build is answered by the assets alone. A request with no file reaches this Worker (run_worker_first is not set):
// it asks the assets, which answer with the 404 page, and when that is a 404 the path is looked up in redirects.json, which
// infra/site/scripts/postbuild.mjs writes: { exact: { "/old": "/new" }, prefixes: [{ from: "/old-folder", to: "/new" }] }. A path that is in
// neither keeps the 404 page. Cloudflare's `_redirects` file is not used: it is not read from this deploy. A browser's navigation request
// (Sec-Fetch-Mode: navigate) reaches this Worker only because main.tf sets the compatibility flag assets_navigation_has_no_effect.

/** The new path for `pathname`, or undefined. A trailing slash does not matter; a prefix matches itself and everything below it. */
export function redirectFor(table, pathname) {
    const path = pathname.length > 1 ? pathname.replace(/\/+$/, "") : pathname;
    if (Object.hasOwn(table.exact, path)) return table.exact[path];
    return table.prefixes.find((rule) => path === rule.from || path.startsWith(`${rule.from}/`))?.to;
}

let table;

export default {
    async fetch(request, env) {
        const response = await env.ASSETS.fetch(request);
        if (response.status !== 404 || (request.method !== "GET" && request.method !== "HEAD")) return response;
        // Read once for the life of this isolate.
        table ??= await env.ASSETS.fetch(new URL("/redirects.json", request.url)).then((answer) => answer.json());
        const target = redirectFor(table, new URL(request.url).pathname);
        return target === undefined ? response : Response.redirect(new URL(target, request.url), 301);
    },
};
