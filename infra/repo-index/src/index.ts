// Browsable index for the package repository bucket.
// Paths ending in "/" list that prefix; every other path is passed through untouched to the bucket's
// custom domain, so dnf, apt and pacman fetch exactly what they fetched before.
const esc = (s) => s.replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]);
const size = (n) => (n < 1024 ? `${n} B` : n < 1048576 ? `${(n / 1024).toFixed(1)} KiB` : `${(n / 1048576).toFixed(1)} MiB`);

export default {
    async fetch(request: Request, env: { REPO: R2Bucket }): Promise<Response> {
        const url = new URL(request.url);
        if (request.method !== "GET" && request.method !== "HEAD") return fetch(request);
        if (!url.pathname.endsWith("/")) return fetch(request);

        const prefix = decodeURIComponent(url.pathname.slice(1));
        const folders = [];
        const files = [];
        let cursor;
        do {
            const page = await env.REPO.list({ prefix, delimiter: "/", cursor, limit: 1000 });
            folders.push(...page.delimitedPrefixes);
            files.push(...page.objects.filter((o) => o.key !== prefix));
            cursor = page.truncated ? page.cursor : undefined;
        } while (cursor);

        if (prefix !== "" && folders.length === 0 && files.length === 0) return new Response("Not found", { status: 404 });

        const rows = [];
        if (prefix !== "") rows.push(`<tr><td><a href="../">../</a></td><td></td><td></td></tr>`);
        for (const f of folders) rows.push(`<tr><td><a href="/${encodeURI(f)}">${esc(f.slice(prefix.length))}</a></td><td></td><td></td></tr>`);
        for (const o of files) {
            const name = o.key.slice(prefix.length);
            rows.push(`<tr><td><a href="/${encodeURI(o.key)}">${esc(name)}</a></td><td>${size(o.size)}</td><td>${o.uploaded.toISOString().slice(0, 16).replace("T", " ")}</td></tr>`);
        }
        const note = prefix === ""
            ? `<p>Signed package repositories for Gnoblin and Bingux. Signing key: <a href="/gpg.key">gpg.key</a>, fingerprint <code>91BA A379 0941 B9B3 BBA6 3BC4 F1B8 D899 DF3D 786E</code>. Install instructions: <a href="https://gnoblin.org">gnoblin.org</a>.</p>`
            : "";
        const html = `<!doctype html><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>Index of /${esc(prefix)}</title>
<style>body{font:15px/1.5 system-ui,sans-serif;max-width:60rem;margin:2rem auto;padding:0 1rem;color:#222;background:#fff}
@media(prefers-color-scheme:dark){body{color:#ddd;background:#16161a}a{color:#8ab4ff}}
table{border-collapse:collapse;width:100%}td{padding:.25rem .75rem .25rem 0;white-space:nowrap}td:first-child{width:100%;white-space:normal;word-break:break-all}
td:nth-child(n+2){color:#888;text-align:right}</style>
<h1>Index of /${esc(prefix)}</h1>${note}<table>${rows.join("")}</table>`;
        return new Response(request.method === "HEAD" ? null : html, { headers: { "content-type": "text/html; charset=utf-8", "cache-control": "public, max-age=60" } });
    },
};
