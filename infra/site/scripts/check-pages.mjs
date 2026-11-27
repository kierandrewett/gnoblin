// Quality gate for the built site: opens key pages in headless Chrome, in light and dark, on a desktop and a phone
// viewport, and fails on console errors, failed requests, horizontal overflow or images that do not load. It also
// checks search, that the docs navigation and outline sit 32 px under the header from the first pixel of scroll,
// that the header is laid out the same on every page, that no docs page overflows at any width from 375 to 1920 px
// (including 1280 to 1324), and the language choice. Run `yarn build`, then `yarn test:pages` (it starts its
// own server on build/).
//
// Options: --base URL (use a running server instead), --shots DIR (save full-page screenshots there).
// CHROME points at the browser (default: google-chrome, google-chrome-stable or chromium on PATH).
import { execFileSync, spawn } from "node:child_process";
import { mkdirSync, readdirSync, statSync } from "node:fs";
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
    "/docs/guides/replace-built-in-dialogs",
    // Language tabs, and the page that leads to the API reference of the three SDKs.
    "/docs/shell-api/idle",
    "/docs/reference",
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

// The API reference of each SDK is the native HTML of its tool, copied beside the pages of the site. Each opens at its stable URL,
// loads everything it names, and has the way back to the docs and to the other two languages.
const REFERENCES = [
    { url: "/reference/typescript/", links: ["/reference/python/", "/reference/rust/"] },
    { url: "/reference/python/", links: ["/reference/typescript/", "/reference/rust/"] },
    { url: "/reference/rust/gnoblin_core/index.html", links: ["/reference/typescript/", "/reference/python/"] },
];
{
    const page = await browser.newPage({ viewport: VIEWPORTS.desktop });
    for (const { url, links } of REFERENCES) {
        const problems = [];
        page.removeAllListeners();
        page.on("pageerror", (err) => problems.push(`page error: ${err.message}`));
        page.on("requestfailed", (req) => problems.push(`request failed: ${req.url()}`));
        page.on("response", (res) => {
            if (res.status() >= 400) problems.push(`${res.status()}: ${res.url()}`);
        });
        await page.goto(base + url, { waitUntil: "networkidle" });
        const hrefs = await page.evaluate(() =>
            [...document.querySelectorAll("a[href]")].map((a) => a.getAttribute("href")),
        );
        for (const href of ["/docs/reference", ...links]) {
            if (!hrefs.includes(href)) problems.push(`no link to ${href}`);
        }
        const title = await page.title();
        if (!/gnoblin/i.test(title)) problems.push(`title is "${title}"`);
        for (const p of problems) failures.push(`${url}: ${p}`);
    }
    // The sidebar of a docs page names the three, and each opens.
    await page.goto(`${base}/docs/reference`, { waitUntil: "networkidle" });
    for (const language of ["TypeScript", "Python", "Rust"]) {
        const link = page.locator(`nav.sidenav a[rel="external"]`, { hasText: language });
        if (!(await link.count())) failures.push(`/docs/reference: the sidebar has no ${language} reference link`);
    }
    await page.close();
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

            // Empty state and suggestions. The query must not start with a letter that some page has as a word of its
            // own: Pagefind matches such a word (`z` in the XR page's `x, y, z` answered `zzzqqq`), so it holds digits.
            await page.keyboard.press("Control+k");
            await box.fill("k8q2v7x1");
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

// Every docs page, light and dark, at the widths where the layout changes: nothing may leave the window, and the page may
// not scroll sideways. The band from 1280 to 1323 px is where the three-column grid and the header once did not fit.
// Code and tables scroll or stack inside their own box; an element that sticks out of its scroller is not counted.
const WIDTHS = [375, 768, 1100, 1280, 1300, 1312, 1323, 1324, 1440, 1920];
const docsPages = ["/docs"];
(function walk(dir) {
    for (const name of readdirSync(dir)) {
        const file = path.join(dir, name);
        if (statSync(file).isDirectory()) walk(file);
        else if (name.endsWith(".html"))
            docsPages.push(
                "/docs/" +
                    path
                        .relative(path.join(site, "build/docs"), file)
                        .replace(/\.html$/, "")
                        .replace(/\/index$/, ""),
            );
    }
})(path.join(site, "build/docs"));
for (const width of WIDTHS) {
    const context = await browser.newContext({ viewport: { width, height: 900 }, reducedMotion: "reduce" });
    const page = await context.newPage();
    for (const url of docsPages) {
        await page.goto(base + url, { waitUntil: "load" });
        for (const scheme of ["light", "dark"]) {
            const found = await page.evaluate((scheme) => {
                document.documentElement.dataset.scheme = scheme;
                const inside = (el) => {
                    for (let p = el.parentElement; p && p !== document.body; p = p.parentElement) {
                        if (getComputedStyle(p).overflowX !== "visible") return true;
                    }
                    return false;
                };
                const out = [];
                for (const el of document.querySelectorAll("body *")) {
                    const r = el.getBoundingClientRect();
                    if (!r.width || !r.height || (r.left >= -1 && r.right <= innerWidth + 1) || inside(el)) continue;
                    out.push(
                        `${el.tagName.toLowerCase()}.${String(el.getAttribute("class") ?? "").split(" ")[0]} ${Math.round(r.left)}..${Math.round(r.right)}`,
                    );
                    if (out.length === 3) break;
                }
                const nav = document.querySelector(".nav-col");
                return {
                    over: document.documentElement.scrollWidth - innerWidth,
                    out,
                    navLeft: nav && getComputedStyle(nav).display !== "none" ? nav.getBoundingClientRect().left : null,
                };
            }, scheme);
            if (found.over > 0) failures.push(`${url} [${width}, ${scheme}] page scrolls sideways by ${found.over}px`);
            if (found.out.length)
                failures.push(`${url} [${width}, ${scheme}] outside the window: ${found.out.join(", ")}`);
            if (found.navLeft !== null && found.navLeft < 0)
                failures.push(`${url} [${width}] sidebar column starts at ${found.navLeft}px`);
        }
    }
    await context.close();
}

// Code never wraps, at any width: every line stays one line, the box scrolls on its own, the page does not, and a box
// whose code fits has no scrollbar.
for (const width of [320, 375, 480, 768, 1280]) {
    const context = await browser.newContext({ viewport: { width, height: 900 }, reducedMotion: "reduce" });
    const page = await context.newPage();
    for (const url of docsPages) {
        await page.goto(base + url, { waitUntil: "networkidle" });
        const found = await page.evaluate(() => {
            const out = [];
            if (document.documentElement.scrollWidth > innerWidth) out.push("the page scrolls sideways");
            for (const pre of document.querySelectorAll(".code pre")) {
                if (!pre.clientWidth) continue;
                const style = getComputedStyle(pre);
                if (style.whiteSpace !== "pre") out.push(`a code box has white-space ${style.whiteSpace}`);
                const lineHeight = parseFloat(style.lineHeight);
                for (const line of pre.querySelectorAll(".line")) {
                    if (line.getBoundingClientRect().height > lineHeight * 1.5) {
                        out.push(`a code line wraps: ${line.textContent.slice(0, 40)}`);
                        break;
                    }
                }
                const fits = pre.scrollWidth <= pre.clientWidth + 1;
                if (fits && pre.offsetHeight - pre.clientHeight > 0) out.push("a code box that fits has a scrollbar");
                if (fits === pre.closest(".code").classList.contains("overflow"))
                    out.push(`a code box fade is ${fits ? "on" : "off"} when its code ${fits ? "fits" : "overflows"}`);
            }
            return out.slice(0, 3);
        });
        if (found.length) failures.push(`${url} [${width}] ${found.join("; ")}`);
    }
    await context.close();
}

// The language choice: stored, or `?lang=`, is applied before the first paint (the page's scripts are blocked here, so
// only app.html and the CSS act), is remembered, and every tab group follows a tab click. Tabs and Copy are 44 px high
// on a phone.
{
    const url = "/docs/shell-api/idle";
    const visibleLanguages = (page) =>
        page.evaluate(() =>
            [...document.querySelectorAll(".lang-tabs")].map(
                (g) =>
                    [...g.querySelectorAll(".lang-panel")].find((p) => getComputedStyle(p).display !== "none")?.dataset
                        .lang,
            ),
        );
    for (const lang of ["python", "rust"]) {
        const context = await browser.newContext({ viewport: VIEWPORTS.desktop });
        await context.addInitScript((value) => localStorage.setItem("gnoblin-language", value), lang);
        const page = await context.newPage();
        await page.route("**/*.js", (route) => route.abort());
        await page.goto(base + url, { waitUntil: "domcontentloaded" });
        const shown = await visibleLanguages(page);
        if (!shown.length || shown.some((l) => l !== lang))
            failures.push(
                `language: with ${lang} stored the groups show ${JSON.stringify(shown)} before any script runs`,
            );
        await context.close();
    }
    {
        const context = await browser.newContext({ viewport: VIEWPORTS.desktop });
        const page = await context.newPage();
        await page.goto(`${base}${url}?lang=rust`, { waitUntil: "networkidle" });
        const stored = await page.evaluate(() => localStorage.getItem("gnoblin-language"));
        if (stored !== "rust") failures.push(`language: ?lang=rust stored "${stored}"`);
        const shown = await visibleLanguages(page);
        if (shown.some((l) => l !== "rust")) failures.push(`language: ?lang=rust shows ${JSON.stringify(shown)}`);
        await page.locator('.lang-tabs [role="tab"][data-lang="python"]').first().click();
        const after = await visibleLanguages(page);
        if (after.some((l) => l !== "python")) failures.push(`language: the tab click left ${JSON.stringify(after)}`);
        await page.goto(`${base}/docs/shell-api/outputs`, { waitUntil: "networkidle" });
        const next = await visibleLanguages(page);
        if (!next.length || next.some((l) => l !== "python"))
            failures.push(`language: the choice was not kept on the next page (${JSON.stringify(next)})`);
        await context.close();
    }
    {
        const context = await browser.newContext({ viewport: VIEWPORTS.phone, hasTouch: true });
        const page = await context.newPage();
        await page.goto(base + url, { waitUntil: "networkidle" });
        const small = await page.evaluate(() =>
            [...document.querySelectorAll(".seg button, .code .copy")]
                .filter((b) => b.getClientRects().length && b.getBoundingClientRect().height < 44)
                .map((b) => `${b.className || b.getAttribute("role")} ${b.getBoundingClientRect().height}px`),
        );
        if (small.length) failures.push(`touch targets under 44px on a phone: ${small.slice(0, 4).join(", ")}`);
        await context.close();
    }
}

// Every language tab is indexed, not only the one shown: a Python or Rust name finds the page, once.
{
    const page = await browser.newPage();
    await page.goto(`${base}/docs/shell-api/idle`, { waitUntil: "networkidle" });
    for (const [query, expected] of [
        ["LockWhenIdle", "/docs/shell-api/idle"],
        ["on_resume", "/docs/shell-api/idle"],
    ]) {
        const urls = await page.evaluate(async (q) => {
            const pagefind = await import("/pagefind/pagefind.js");
            const found = await pagefind.search(q);
            return (await Promise.all(found.results.map((r) => r.data()))).map((d) => d.url.replace(/\.html$/, ""));
        }, query);
        if (!urls.includes(expected))
            failures.push(`search: "${query}" does not find ${expected} (${urls.slice(0, 3)})`);
        if (new Set(urls).size !== urls.length) failures.push(`search: "${query}" lists a page twice`);
    }
    await page.close();
}

// The docs navigation and outline sit 32 px under the header from scroll 0, so nothing moves when the reader
// starts to scroll, and stay there on a long page.
{
    const page = await browser.newPage({ viewport: VIEWPORTS.desktop });
    await page.goto(`${base}/docs/bridge/connection`, { waitUntil: "networkidle" });
    const tops = [];
    for (const y of [0, 80, 1500, 3000]) {
        const at = await page.evaluate((y) => {
            window.scrollTo(0, y);
            const top = (sel) => document.querySelector(sel).getBoundingClientRect().top;
            return {
                y: window.scrollY,
                header: top("header.topbar") + document.querySelector("header.topbar").offsetHeight,
                nav: top(".nav-sticky"),
                toc: top("aside.toc"),
            };
        }, y);
        tops.push(`${at.y}px: nav ${at.nav}px, outline ${at.toc}px`);
        const expected = at.header + 32;
        if (at.y !== y) failures.push(`sticky: page too short to scroll to ${y}px`);
        for (const [name, value] of [
            ["navigation", at.nav],
            ["outline", at.toc],
        ]) {
            if (Math.abs(value - expected) > 0.5) {
                failures.push(`sticky: ${name} top is ${value}px at ${y}px of scroll, expected ${expected}px`);
            }
        }
    }
    console.log(`Navigation and outline top by scroll position: ${tops.join("; ")}`);
    await page.close();
}

// One header: at each width, its height and the x of the mark, the site links, search and the right-hand controls
// are the same on every page.
{
    const routes = [
        "/",
        "/install",
        "/bingux",
        "/docs",
        "/docs/bridge/connection",
        "/credits",
        "/this-page-does-not-exist",
    ];
    for (const width of [1440, 1100, 900, 390]) {
        const page = await browser.newPage({ viewport: { width, height: 900 } });
        let reference = null;
        for (const url of routes) {
            await page.goto(base + url, { waitUntil: "networkidle" });
            const geometry = await page.evaluate(() => {
                const header = document.querySelector("header.topbar");
                const xs = (sel) =>
                    [...header.querySelectorAll(sel)]
                        .map((el) => el.getBoundingClientRect())
                        .filter((r) => r.width > 0)
                        .map((r) => Math.round(r.x * 10) / 10);
                return {
                    height: header.getBoundingClientRect().height,
                    logo: xs(".brand"),
                    nav: xs(".site a"),
                    search: xs(".search-field"),
                    right: xs(".proj a, .compact > *"),
                };
            });
            if (shots) {
                await page.screenshot({
                    path: path.join(shots, `header-${width}-${url.replace(/^\/$/, "home").replace(/\//g, "_")}.png`),
                    clip: { x: 0, y: 0, width, height: 64 },
                });
            }
            if (!reference) {
                reference = geometry;
                console.log(`Header at ${width}px: ${JSON.stringify(geometry)}`);
            } else if (JSON.stringify(geometry) !== JSON.stringify(reference)) {
                failures.push(
                    `header: ${url} at ${width}px is ${JSON.stringify(geometry)}, / is ${JSON.stringify(reference)}`,
                );
            }
        }
        await page.close();
    }
}

await browser.close();
server?.kill();

if (failures.length) {
    console.error(failures.join("\n"));
    console.error(`\n${failures.length} problem(s)`);
    process.exit(1);
}
console.log(`Checked ${PAGES.length} pages in ${Object.keys(VIEWPORTS).length} sizes, light and dark, and search.`);
