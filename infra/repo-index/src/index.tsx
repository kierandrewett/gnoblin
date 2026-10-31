// Browsable index for the package repository bucket.
// Paths ending in "/" list that folder. Every other path is passed to the bucket's custom domain unchanged, so dnf,
// apt and pacman fetch exactly what they fetched before.
// The page follows the plain listing at https://archive.mozilla.org/pub/. It has no stylesheet. The color-scheme meta
// tag makes the browser use its own light or dark defaults.
import type { FC } from "hono/jsx";

type Entry = { type: "Dir" | "File"; name: string; href: string; size?: number; uploaded?: Date };

const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

// 234K, 1.5M, 2.1G. A size under 1024 bytes has no suffix.
const formatSize = (bytes: number): string => {
    if (bytes < 1024) return String(bytes);
    if (bytes < 1048576) return `${Math.round(bytes / 1024)}K`;
    if (bytes < 1073741824) return `${(bytes / 1048576).toFixed(1).replace(/\.0$/, "")}M`;
    return `${(bytes / 1073741824).toFixed(1).replace(/\.0$/, "")}G`;
};

// 13-Feb-2023 04:21, in UTC.
const formatDate = (date: Date): string => {
    const pad = (value: number): string => String(value).padStart(2, "0");
    return `${pad(date.getUTCDate())}-${MONTHS[date.getUTCMonth()]}-${date.getUTCFullYear()} ${pad(date.getUTCHours())}:${pad(date.getUTCMinutes())}`;
};

const Row: FC<{ entry: Entry }> = ({ entry }) => (
    <tr>
        <td>{entry.type}</td>
        <td>
            <a href={entry.href}>{entry.name}</a>
        </td>
        <td>{entry.size === undefined ? "" : formatSize(entry.size)}</td>
        <td>{entry.uploaded ? formatDate(entry.uploaded) : ""}</td>
    </tr>
);

const parentOf = (prefix: string): string => {
    const parents = prefix.split("/").slice(0, -2);
    return parents.length === 0 ? "/" : `/${parents.join("/")}/`;
};

const Index: FC<{ prefix: string; entries: Entry[] }> = ({ prefix, entries }) => (
    <html lang="en">
        <head>
            <meta charset="utf-8" />
            <meta name="color-scheme" content="light dark" />
            <title>Directory Listing: /{prefix}</title>
        </head>
        <body>
            <h1>Index of /{prefix}</h1>
            <table>
                <tr>
                    <th>Type</th>
                    <th>Name</th>
                    <th>Size</th>
                    <th>Last Modified</th>
                </tr>
                {prefix === "" ? null : <Row entry={{ type: "Dir", name: "..", href: parentOf(prefix) }} />}
                {entries.map((entry) => (
                    <Row entry={entry} />
                ))}
            </table>
        </body>
    </html>
);

const NotFound: FC<{ path: string }> = ({ path }) => (
    <html lang="en">
        <head>
            <meta charset="utf-8" />
            <meta name="color-scheme" content="light dark" />
            <title>404 Not Found</title>
        </head>
        <body>
            <h1>Not Found</h1>
            <p>The requested URL {path} was not found on this server.</p>
            <p>
                <a href="/">Back to the repository index</a>
            </p>
        </body>
    </html>
);

const htmlResponse = (request: Request, status: number, html: string, cacheControl: string): Response =>
    new Response(request.method === "HEAD" ? null : `<!doctype html>${html}`, {
        status,
        headers: { "content-type": "text/html; charset=utf-8", "cache-control": cacheControl },
    });

const notFound = (request: Request): Response =>
    htmlResponse(request, 404, (<NotFound path={new URL(request.url).pathname} />).toString(), "no-store");

export default {
    async fetch(request: Request, env: { REPO: R2Bucket }): Promise<Response> {
        const url = new URL(request.url);
        if ((request.method !== "GET" && request.method !== "HEAD") || !url.pathname.endsWith("/")) {
            const upstream = await fetch(request);
            return upstream.status === 404 && (request.method === "GET" || request.method === "HEAD")
                ? notFound(request)
                : upstream;
        }

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

        if (prefix !== "" && folders.length === 0 && files.length === 0) return notFound(request);

        const entries: Entry[] = [
            ...folders.map((folder) => ({
                type: "Dir" as const,
                name: folder.slice(prefix.length),
                href: `/${encodeURI(folder)}`,
            })),
            ...files.map((object) => ({
                type: "File" as const,
                name: object.key.slice(prefix.length),
                href: `/${encodeURI(object.key)}`,
                size: object.size,
                uploaded: object.uploaded,
            })),
        ];

        return htmlResponse(
            request,
            200,
            (<Index prefix={prefix} entries={entries} />).toString(),
            "public, max-age=60",
        );
    },
};
