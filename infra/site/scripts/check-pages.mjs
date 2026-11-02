// Quality gate for the built site: opens key pages in headless Chrome, in light and dark, on a desktop and a phone
// viewport, and fails on console errors, failed requests, horizontal overflow or images that do not load. It also
// checks that search finds a page and that the docs outline sticks under the header. Run `yarn build`, then
// `yarn test:pages` (it starts its own server on build/).
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
    "/docs/bridge/connection",
    "/docs/install-arch",
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

// Search. From the home page and from a docs page, at desktop and phone sizes, light and dark: open the dialog, type,
// and expect results that show a page title, its section trail, a count and a matching heading; open a heading and
// land on that heading; check the empty state, the suggestions, Esc and where focus goes after the dialog closes.
const SUGGESTIONS = ["install fedora", "keyboard shortcuts", "window rules", "mouse", "animations"];
for (const [vpName, viewport] of [
    ["desktop", VIEWPORTS.desktop],
    ["phone", VIEWPORTS.phone],
]) {
    for (const scheme of ["light", "dark"]) {
        const context = await browser.newContext({ viewport, colorScheme: scheme, reducedMotion: "reduce" });
        const page = await context.newPage();
        const fail = (message) => failures.push(`search [${vpName}, ${scheme}]: ${message}`);
        const problems = [];
        page.on("console", (msg) => msg.type() === "error" && problems.push(`console: ${msg.text()}`));
        page.on("requestfailed", (req) => problems.push(`request failed: ${req.url()}`));
        try {
            const box = page.getByRole("combobox", { name: "Search the docs" });
            const options = page.locator('#search-results [role="option"]');

            // The search button opens the dialog; Esc closes it and focus goes back to the button.
            await page.goto(`${base}/`, { waitUntil: "networkidle" });
            if (vpName === "desktop") {
                const button = page.getByRole("button", { name: "Search the docs" }).first();
                await button.click();
                await box.waitFor({ timeout: 5000 });
                if (!(await box.evaluate((el) => el === document.activeElement))) fail("input is not focused on open");
                await page.keyboard.press("Escape");
                await page.locator("dialog.search[open]").waitFor({ state: "detached", timeout: 5000 });
                if (!(await button.evaluate((el) => el === document.activeElement)))
                    fail("focus did not return to the button");
            }

            // Results: title, trail, excerpt, count.
            await page.goto(`${base}/docs`, { waitUntil: "networkidle" });
            await page.keyboard.press("Control+k");
            await box.fill("mouse");
            await options.first().waitFor({ timeout: 10000 });
            const first = options.first();
            if (!(await first.locator(".rt").innerText()).trim()) fail("first result has no title");
            const trails = await page.locator("#search-results .page .rp").allInnerTexts();
            if (!trails.some((t) => t.includes(" › ")))
                fail(`no result shows a section trail (${JSON.stringify(trails)})`);
            if (!(await page.locator("#search-results .ex mark").count())) fail("no excerpt has a highlighted match");
            const summary = (await page.locator(".summary").innerText()).trim();
            if (!/^\d+ results?/.test(summary)) fail(`result count is "${summary}"`);
            if (!(await page.locator("#search-results .heading").count())) fail("no result lists a matching heading");
            if (shots) await page.screenshot({ path: path.join(shots, `search-${vpName}-${scheme}.png`) });

            // A heading result opens the page at that heading.
            const href = await page.evaluate(() => {
                const row = document.querySelector("#search-results .heading");
                return row?.id ?? null;
            });
            if (href) {
                const index = Number(href.replace("search-hit-", ""));
                for (let i = 0; i < index; i++) await page.keyboard.press("ArrowDown");
                await page.keyboard.press("Enter");
                await page.waitForURL((u) => u.hash !== "" && u.pathname !== "/docs", { timeout: 10000 });
                const hash = decodeURIComponent(new URL(page.url()).hash.slice(1));
                const target = await page.evaluate((id) => !!document.getElementById(id), hash);
                if (!target) fail(`heading link ${page.url()} has no element with id "${hash}"`);
            }

            // Empty state and suggestions.
            await page.keyboard.press("Control+k");
            await box.fill("zzzqqq");
            await page.getByText("No results for").waitFor({ timeout: 10000 });
            for (const text of SUGGESTIONS) {
                await box.fill(text);
                try {
                    await options.first().waitFor({ timeout: 10000 });
                } catch {
                    fail(`suggestion "${text}" finds nothing`);
                }
            }
            await box.fill("");
            await page.getByRole("button", { name: SUGGESTIONS[0] }).click();
            await options.first().waitFor({ timeout: 10000 });
            await page.keyboard.press("Escape");
        } catch (error) {
            fail(error.message.split("\n")[0]);
        }
        for (const p of problems) fail(p);
        await context.close();
    }
}

// The docs outline sticks 32 px under the header: scroll a long page and expect its top to stay put.
{
    const page = await browser.newPage({ viewport: VIEWPORTS.desktop });
    await page.goto(`${base}/docs/bridge/connection`, { waitUntil: "networkidle" });
    const tops = [];
    for (const y of [0, 1500, 3000]) {
        const at = await page.evaluate((y) => {
            window.scrollTo(0, y);
            const header = document.querySelector("header.topbar").getBoundingClientRect().height;
            const toc = document.querySelector("aside.toc").getBoundingClientRect().top;
            return { y: window.scrollY, header, toc };
        }, y);
        tops.push(`${at.y}px: ${at.toc}px`);
        if (y === 0) continue;
        if (at.y !== y) failures.push(`outline: page too short to scroll to ${y}px`);
        else if (Math.abs(at.toc - (at.header + 32)) > 0.5) {
            failures.push(`outline: top is ${at.toc}px at ${y}px of scroll, expected ${at.header + 32}px`);
        }
    }
    console.log(`Outline top by scroll position: ${tops.join(", ")}`);
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
