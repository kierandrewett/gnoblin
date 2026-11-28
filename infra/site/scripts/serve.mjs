// Serves build/ the way the Cloudflare Worker's static assets do (infra/terraform/main.tf): html_handling
// "auto-trailing-slash" serves /name from name.html or name/index.html, and not_found_handling "404-page" answers
// unknown paths with 404.html and status 404, except the old URLs of redirects.json, which get a 301 (the Worker, infra/terraform/worker.js). Usage: yarn preview [--port 4173]
import { createReadStream, existsSync, readFileSync, statSync } from "node:fs";
import { createServer } from "node:http";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { redirectFor } from "../../terraform/worker.js";

const root = fileURLToPath(new URL("../build", import.meta.url));
const portArg = process.argv.indexOf("--port");
const port = Number(portArg === -1 ? (process.env.PORT ?? 4173) : process.argv[portArg + 1]);

const TYPES = {
    ".html": "text/html; charset=utf-8",
    ".js": "text/javascript",
    ".css": "text/css",
    ".json": "application/json",
    ".svg": "image/svg+xml",
    ".png": "image/png",
    ".webp": "image/webp",
    ".wasm": "application/wasm",
    ".COPYING": "text/plain; charset=utf-8",
};

const isFile = (p) => existsSync(p) && statSync(p).isFile();

// The Worker's other job (infra/terraform/worker.js): a path with no page is looked up in redirects.json and answered with a 301.
const table = existsSync(path.join(root, "redirects.json"))
    ? JSON.parse(readFileSync(path.join(root, "redirects.json"), "utf8"))
    : { exact: {}, prefixes: [] };

function notFound(res, pathname) {
    const target = redirectFor(table, pathname);
    if (target === undefined) return send(res, 404, path.join(root, "404.html"));
    res.writeHead(301, { Location: target });
    return res.end();
}

function send(res, status, file, headers = {}) {
    res.writeHead(status, { "Content-Type": TYPES[path.extname(file)] ?? "application/octet-stream", ...headers });
    createReadStream(file).pipe(res);
}

createServer((req, res) => {
    const url = new URL(req.url ?? "/", "http://localhost");
    const pathname = decodeURIComponent(url.pathname);
    const file = path.join(root, pathname);
    if (!file.startsWith(root)) return notFound(res, pathname);

    if (isFile(file)) {
        // A request for name.html redirects to /name, as auto-trailing-slash does.
        if (pathname.endsWith(".html")) {
            const clean = pathname.replace(/(\/index)?\.html$/, "") || "/";
            res.writeHead(307, { Location: clean + url.search });
            return res.end();
        }
        return send(res, 200, file);
    }
    const bare = pathname.replace(/\/$/, "");
    if (pathname !== "/" && isFile(path.join(root, `${bare}.html`))) {
        if (pathname.endsWith("/")) {
            res.writeHead(307, { Location: bare + url.search });
            return res.end();
        }
        return send(res, 200, path.join(root, `${bare}.html`));
    }
    if (isFile(path.join(root, bare, "index.html"))) {
        if (!pathname.endsWith("/")) {
            res.writeHead(307, { Location: `${bare}/${url.search}` });
            return res.end();
        }
        return send(res, 200, path.join(root, bare, "index.html"));
    }
    return notFound(res, pathname);
}).listen(port, () => console.log(`Serving build/ at http://localhost:${port}`));
