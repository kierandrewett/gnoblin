// Quality gate for the built site: opens key pages in headless Chrome, in light and dark, on a desktop and a phone
// viewport, and fails on console errors, failed requests, horizontal overflow or images that do not load. It also
// checks that search finds a page. Run `yarn build`, then `yarn test:pages` (it starts its own server on build/).
//
// Options: --base URL (use a running server instead), --shots DIR (save full-page screenshots there).
// CHROME points at the browser (default: google-chrome, google-chrome-stable or chromium on PATH).
import { execFileSync, spawn } from "node:child_process";
import { mkdirSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { chromium } from "playwright-core";

const site = fileURLToPath(new URL("..", import.meta.url));
const arg = (name) => {
    const i = process.argv.indexOf(name);
    return i === -1 ? undefined : process.argv[i + 1];
};

const PAGES = [
    "/",
    "/install",
    "/bingux",
    "/credits",
    "/docs",
    "/docs/install-fedora",
    "/docs/config/configure/input/mouse",
    "/docs/installation",
    "/docs/optional-packages",
    "/docs/bridge/examples",
    "/this-page-does-not-exist",
];
const VIEWPORTS = {
    desktop: { width: 1440, height: 900 },
    tablet: { width: 900, height: 1100 },
    phone: { width: 390, height: 844 },
};

let server;
let base = arg("--base");
if (!base) {
    const port = 4300 + Math.floor(Math.random() * 500);
    server = spawn(process.execPath, [path.join(site, "scripts/serve.mjs"), "--port", String(port)], {
        stdio: "ignore",
    });
    base = `http://localhost:${port}`;
    await new Promise((resolve) => setTimeout(resolve, 500));
}
const shots = arg("--shots");
if (shots) mkdirSync(shots, { recursive: true });

const chrome =
    process.env.CHROME ??
    execFileSync("sh", ["-c", "command -v google-chrome || command -v google-chrome-stable || command -v chromium"], {
        encoding: "utf8",
    }).trim();
const browser = await chromium.launch({ executablePath: chrome, headless: true });
const failures = [];

for (const [vpName, viewport] of Object.entries(VIEWPORTS)) {
    for (const scheme of ["light", "dark"]) {
        // Reduced motion: the bands' scroll-driven fade would leave unscrolled parts of full-page screenshots blank.
        const context = await browser.newContext({
            viewport,
            colorScheme: scheme,
            reducedMotion: "reduce",
            deviceScaleFactor: 1,
        });
        for (const url of PAGES) {
            const page = await context.newPage();
            const problems = [];
            // The not-found page answers with 404 on purpose; Chrome logs that once.
            let expected404 = url === "/this-page-does-not-exist";
            page.on("console", (msg) => {
                if (msg.type() !== "error") return;
                if (expected404 && msg.text().includes("status of 404")) {
                    expected404 = false;
                    return;
                }
                problems.push(`console: ${msg.text()}`);
            });
            page.on("pageerror", (err) => problems.push(`page error: ${err.message}`));
            page.on("requestfailed", (req) => problems.push(`request failed: ${req.url()}`));
            page.on("response", (res) => {
                // The 404 page is expected to answer with 404; nothing else should.
                if (res.status() >= 400 && res.url() !== base + url) problems.push(`${res.status()}: ${res.url()}`);
            });

            await page.goto(base + url, { waitUntil: "networkidle" });
            const result = await page.evaluate(async () => {
                for (const img of document.images) img.loading = "eager";
                await Promise.all(
                    [...document.images].map((img) =>
                        img.complete ? null : new Promise((resolve) => (img.onload = img.onerror = resolve)),
                    ),
                );
                // Decode before the screenshot so lazily decoded images are painted.
                await Promise.all([...document.images].map((img) => img.decode().catch(() => null)));
                return {
                    scheme: document.documentElement.dataset.scheme,
                    overflow: document.documentElement.scrollWidth - window.innerWidth,
                    broken: [...document.images].filter((img) => !img.naturalWidth).map((img) => img.currentSrc),
                };
            });
            if (result.scheme !== scheme) problems.push(`scheme is ${result.scheme}, expected ${scheme}`);
            if (result.overflow > 0) problems.push(`horizontal overflow of ${result.overflow}px`);
            for (const src of result.broken) problems.push(`image did not load: ${src}`);

            if (shots) {
                const name = `${url.replace(/^\/$/, "home").replace(/^\//, "").replace(/\//g, "_")}-${vpName}-${scheme}`;
                await page.screenshot({ path: path.join(shots, `${name}.png`), fullPage: true });
            }
            for (const p of problems) failures.push(`${url} [${vpName}, ${scheme}] ${p}`);
            await page.close();
        }
        await context.close();
    }
}

// Search: open with Ctrl+K on a docs page, type, and expect a result that opens a page.
{
    const page = await browser.newPage({ viewport: VIEWPORTS.desktop });
    await page.goto(`${base}/docs`, { waitUntil: "networkidle" });
    await page.keyboard.press("Control+k");
    await page.getByRole("combobox", { name: "Search the docs" }).fill("mouse");
    try {
        await page.getByRole("option").first().waitFor({ timeout: 10000 });
        if (shots) await page.screenshot({ path: path.join(shots, "search-desktop-light.png") });
        await page.keyboard.press("Enter");
        await page.waitForURL((u) => u.pathname !== "/docs", { timeout: 10000 });
    } catch (error) {
        failures.push(`search: ${error.message.split("\n")[0]}`);
    }
    await page.close();
}

await browser.close();
server?.kill();

if (failures.length) {
    console.error(failures.join("\n"));
    console.error(`\n${failures.length} problem(s)`);
    process.exit(1);
}
console.log(`Checked ${PAGES.length} pages in ${Object.keys(VIEWPORTS).length} sizes, light and dark, and search.`);
