// Browsable index for the package repository bucket.
// Paths ending in "/" list that folder. Every other path is passed to the bucket's custom domain unchanged, so dnf,
// apt and pacman fetch exactly what they fetched before.
import type { FC } from "hono/jsx";

type Entry = { name: string; href: string; size?: number; uploaded?: Date };

const KEY_FINGERPRINT = "91BA A379 0941 B9B3 BBA6 3BC4 F1B8 D899 DF3D 786E";

const formatSize = (bytes: number): string =>
    bytes < 1024 ? `${bytes} B` : bytes < 1048576 ? `${(bytes / 1024).toFixed(1)} KiB` : `${(bytes / 1048576).toFixed(1)} MiB`;

const style = `
body { font: 15px/1.5 system-ui, sans-serif; max-width: 60rem; margin: 2rem auto; padding: 0 1rem; color: #222; background: #fff; }
@media (prefers-color-scheme: dark) { body { color: #ddd; background: #16161a; } a { color: #8ab4ff; } }
table { border-collapse: collapse; width: 100%; }
td { padding: 0.25rem 0.75rem 0.25rem 0; white-space: nowrap; }
td:first-child { width: 100%; white-space: normal; word-break: break-all; }
td:nth-child(n + 2) { color: #888; text-align: right; }
`;

const Intro: FC = () => (
    <p>
        Signed package repositories for Gnoblin and Bingux. Signing key: <a href="/gpg.key">gpg.key</a>, fingerprint{" "}
        <code>{KEY_FINGERPRINT}</code>. Install instructions: <a href="https://gnoblin.org">gnoblin.org</a>.
    </p>
);

const Row: FC<{ entry: Entry }> = ({ entry }) => (
    <tr>
        <td>
            <a href={entry.href}>{entry.name}</a>
        </td>
        <td>{entry.size === undefined ? "" : formatSize(entry.size)}</td>
        <td>{entry.uploaded ? entry.uploaded.toISOString().slice(0, 16).replace("T", " ") : ""}</td>
    </tr>
);

const Index: FC<{ prefix: string; entries: Entry[] }> = ({ prefix, entries }) => (
    <html lang="en">
        <head>
            <meta charset="utf-8" />
            <meta name="viewport" content="width=device-width,initial-scale=1" />
            <title>Index of /{prefix}</title>
            <style>{style}</style>
        </head>
        <body>
            <h1>Index of /{prefix}</h1>
            {prefix === "" ? <Intro /> : null}
            <table>
                {prefix === "" ? null : <Row entry={{ name: "../", href: "../" }} />}
                {entries.map((entry) => (
                    <Row entry={entry} />
                ))}
            </table>
        </body>
    </html>
);

export default {
    async fetch(request: Request, env: { REPO: R2Bucket }): Promise<Response> {
        const url = new URL(request.url);
        if ((request.method !== "GET" && request.method !== "HEAD") || !url.pathname.endsWith("/")) return fetch(request);

        const prefix = decodeURIComponent(url.pathname.slice(1));
        const folders: string[] = [];
        const files: R2Object[] = [];
        let cursor: string | undefined;
        do {
            const page = await env.REPO.list({ prefix, delimiter: "/", cursor, limit: 1000 });
            folders.push(...page.delimitedPrefixes);
            files.push(...page.objects.filter((object) => object.key !== prefix));
            cursor = page.truncated ? page.cursor : undefined;
        } while (cursor);

        if (prefix !== "" && folders.length === 0 && files.length === 0) return new Response("Not found", { status: 404 });

        const entries: Entry[] = [
            ...folders.map((folder) => ({ name: folder.slice(prefix.length), href: `/${encodeURI(folder)}` })),
            ...files.map((object) => ({
                name: object.key.slice(prefix.length),
                href: `/${encodeURI(object.key)}`,
                size: object.size,
                uploaded: object.uploaded,
            })),
        ];

        const html = `<!doctype html>${(<Index prefix={prefix} entries={entries} />).toString()}`;
        return new Response(request.method === "HEAD" ? null : html, {
            headers: { "content-type": "text/html; charset=utf-8", "cache-control": "public, max-age=60" },
        });
    },
};
