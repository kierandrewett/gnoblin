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
import { mkdirSync, readdirSync, readFileSync, statSync } from "node:fs";
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
    // Language tabs, and the API reference.
    "/docs/shell-api/idle",
    "/reference",
    "/reference/core/client",
    "/reference/std/workspace",
    "/reference/errors",
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
const WIDTHS = [320, 375, 480, 768, 1100, 1280, 1300, 1312, 1323, 1324, 1440, 1920];
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
                // The controls of each code header (tabs, Copy) must sit inside the code box: the box clips them.
                const clipped = [];
                for (const box of document.querySelectorAll(".code")) {
                    const frame = box.getBoundingClientRect();
                    if (!frame.width) continue;
                    for (const control of box.querySelectorAll(".code-head button")) {
                        const r = control.getBoundingClientRect();
                        if (!r.width) continue;
                        if (r.left < frame.left - 0.5 || r.right > frame.right + 0.5)
                            clipped.push(
                                `${control.textContent.trim() || control.className} ${Math.round(r.left - frame.left)}..${Math.round(r.right - frame.left)} of ${Math.round(frame.width)}`,
                            );
                    }
                    if (clipped.length >= 3) break;
                }
                const nav = document.querySelector(".nav-col");
                return {
                    clipped,
                    over: document.documentElement.scrollWidth - innerWidth,
                    out,
                    navLeft: nav && getComputedStyle(nav).display !== "none" ? nav.getBoundingClientRect().left : null,
                };
            }, scheme);
            if (found.clipped.length)
                failures.push(
                    `${url} [${width}, ${scheme}] code header controls leave the code box: ${found.clipped.join(", ")}`,
                );
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
    const routes = ["/", "/bingux", "/docs", "/docs/bridge/connection", "/credits", "/this-page-does-not-exist"];
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

// What the reference checks need from the built site: the largest class pages (the ones most likely to overflow), a page of types and an
// event whose name is spelled differently in the three languages.
const referenceRoot = path.join(site, "build/reference");
const referenceClasses = ["core", "std"].flatMap((pkg) =>
    readdirSync(path.join(referenceRoot, pkg))
        .filter((name) => name.endsWith(".html"))
        .map((name) => ({
            url: `/reference/${pkg}/${name.replace(/\.html$/, "")}`,
            size: statSync(path.join(referenceRoot, pkg, name)).size,
        })),
);
const largestClassPages = referenceClasses
    .sort((a, b) => b.size - a.size)
    .map((c) => c.url)
    .filter((url) => !["/reference/core/client", "/reference/std/workspace", "/reference/core/output"].includes(url))
    .slice(0, 3);
const typesPageUrl = `/reference/core/types/${readdirSync(path.join(referenceRoot, "core/types"))
    .find((name) => name.endsWith(".html"))
    ?.replace(/\.html$/, "")}`;
const clientEvents = [
    ...readFileSync(path.join(referenceRoot, "core/client.html"), "utf8").matchAll(/ id="event-([a-z0-9_]+)"/g),
].map((m) => m[1]);
const firstEvent = clientEvents.find((name) => name.includes("_")) ?? clientEvents[0];
if (!firstEvent) failures.push("reference: /reference/core/client has no event");

// The API reference (/reference): built from reference.json by the site itself, three languages in one page. Every check below opens the
// built pages in Chrome. The routes are the index, a class of each library, a page of types, the errors and the page a member's anchor names.
const REFERENCE_PAGES = [
    "/reference",
    "/reference/core/client",
    "/reference/std/workspace",
    "/reference/core/output#apply",
    "/reference/errors",
    ...(typesPageUrl ? [typesPageUrl] : []),
];
const LANGS = ["typescript", "python", "rust"];

// 1. Every page of the reference opens in each language without a console error, a failed request, a missing image or a sideways scroll,
//    at the widths where the layout changes: a phone, the band from 1280 to 1323 px (where three columns and the header once did not fit)
//    and a wide desktop. Code and tables scroll or stack inside their own box; an element that sticks out of its scroller does not count.
for (const width of [375, 1280, 1300, 1312, 1323, 1324, 1440]) {
    for (const lang of LANGS) {
        const context = await browser.newContext({ viewport: { width, height: 900 }, reducedMotion: "reduce" });
        await context.addInitScript((value) => localStorage.setItem("gnoblin-language", value), lang);
        const page = await context.newPage();
        const problems = [];
        page.on("console", (msg) => msg.type() === "error" && problems.push(`console: ${msg.text()}`));
        page.on("pageerror", (err) => problems.push(`page error: ${err.message}`));
        page.on("requestfailed", (req) => problems.push(`request failed: ${req.url()}`));
        for (const url of [...REFERENCE_PAGES, ...largestClassPages]) {
            problems.length = 0;
            await page.goto(base + url, { waitUntil: "load" });
            const found = await page.evaluate(() => {
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
                return { over: document.documentElement.scrollWidth - innerWidth, out };
            });
            const where = `${url} [${width}, ${lang}]`;
            if (found.over > 0) failures.push(`${where} the page scrolls sideways by ${found.over}px`);
            if (found.out.length) failures.push(`${where} outside the window: ${found.out.join(", ")}`);
            for (const p of problems) failures.push(`${where} ${p}`);
        }
        await context.close();
    }
}

// 2. Structure of a page, in each language: one h1 and a heading outline that never skips a level, every table with a heading cell for each
//    column and a row heading, every anchor reachable by the keyboard, and a title.
{
    const context = await browser.newContext({ viewport: VIEWPORTS.desktop, reducedMotion: "reduce" });
    const page = await context.newPage();
    for (const url of REFERENCE_PAGES.map((u) => u.split("#")[0]).concat(largestClassPages)) {
        await page.goto(base + url, { waitUntil: "load" });
        for (const lang of LANGS) {
            await page.evaluate((l) => (document.documentElement.dataset.lang = l), lang);
            const found = await page.evaluate(() => {
                const shown = (el) => el.getClientRects().length > 0;
                const problems = [];
                const headings = [...document.querySelectorAll("main h1, main h2, main h3, main h4")].filter(shown);
                if (headings.filter((h) => h.tagName === "H1").length !== 1)
                    problems.push("the page does not have exactly one h1");
                let last = 0;
                for (const h of headings) {
                    const level = Number(h.tagName[1]);
                    if (last && level > last + 1)
                        problems.push(
                            `the outline jumps from h${last} to h${level} at "${h.textContent.trim().slice(0, 30)}"`,
                        );
                    last = level;
                }
                for (const table of document.querySelectorAll("table.ref-tbl")) {
                    if (!shown(table)) continue;
                    const heads = [...table.querySelectorAll("thead th")].filter(shown);
                    if (heads.some((th) => th.getAttribute("scope") !== "col"))
                        problems.push("a column heading has no scope=col");
                    const rows = [...table.querySelectorAll("tbody tr")].filter(shown);
                    if (
                        rows.some(
                            (row) =>
                                ![...row.children].some(
                                    (c) => shown(c) && c.tagName === "TH" && c.getAttribute("scope") === "row",
                                ),
                        )
                    )
                        problems.push("a row has no row heading");
                    const cells = rows[0] ? [...rows[0].children].filter(shown).length : heads.length;
                    if (heads.length !== cells)
                        problems.push(`a table has ${heads.length} column headings and ${cells} cells in a row`);
                }
                for (const a of document.querySelectorAll("main a.anchor")) {
                    if (shown(a) && a.tabIndex < 0) problems.push("an anchor is not reachable by the keyboard");
                }
                return problems.slice(0, 3);
            });
            for (const p of found) failures.push(`${url} [${lang}] ${p}`);
        }
        if (!/Gnoblin/.test(await page.title())) failures.push(`${url}: the title is "${await page.title()}"`);
    }
    await context.close();
}

// 3. The language picker: a radio group with one tab stop, the arrow keys move and choose, the choice is kept and `?lang=` wins once. It
//    changes spelling only: the URL, the anchors, the headings and the rows are the same in all three languages, the names are not.
{
    const url = "/reference/core/client";
    const context = await browser.newContext({ viewport: VIEWPORTS.desktop, reducedMotion: "reduce" });
    const page = await context.newPage();
    await page.goto(base + url, { waitUntil: "networkidle" });
    const snapshot = () =>
        page.evaluate(() => {
            const shown = (el) => el.getClientRects().length > 0;
            const names = [...document.querySelectorAll("#sec-properties ~ .tbl-wrap tr[id] > th[scope=row]")]
                .filter(shown)
                .map((th) => th.innerText.trim());
            return {
                path: location.pathname,
                lang: document.documentElement.dataset.lang ?? "typescript",
                ids: [...document.querySelectorAll("main [id]")].map((el) => el.id).sort(),
                headings: [...document.querySelectorAll("main h2")].map((h) => h.id),
                rows: document.querySelectorAll("#sec-properties ~ .tbl-wrap tbody tr").length,
                names,
                hrefs: [...document.querySelectorAll("main a.ty")].map((a) => a.getAttribute("href")).sort(),
            };
        });
    const radios = page.getByRole("radio");
    if ((await radios.count()) !== 3) failures.push(`picker: ${await radios.count()} radios, expected 3`);
    const tabStops = await page.evaluate(
        () =>
            [...document.querySelectorAll('[role="radiogroup"] [role="radio"]')].filter((r) => r.tabIndex === 0).length,
    );
    if (tabStops !== 1) failures.push(`picker: ${tabStops} tab stops, expected 1`);
    const before = await snapshot();
    const shownBy = {};
    for (const [lang, label] of [
        ["typescript", "TypeScript"],
        ["python", "Python"],
        ["rust", "Rust"],
    ]) {
        await page.getByRole("radio", { name: label }).click();
        const now = await snapshot();
        shownBy[lang] = now;
        if (now.lang !== lang) failures.push(`picker: ${label} left <html data-lang> at ${now.lang}`);
        if (
            !(await page
                .getByRole("radio", { name: label })
                .evaluate((el) => el.getAttribute("aria-checked") === "true"))
        )
            failures.push(`picker: ${label} is not aria-checked after a click`);
        if (now.path !== before.path) failures.push(`picker: ${label} changed the URL to ${now.path}`);
        for (const key of ["ids", "headings", "hrefs"]) {
            if (JSON.stringify(now[key]) !== JSON.stringify(before[key]))
                failures.push(`picker: ${label} changed the page's ${key}`);
        }
        if (now.rows !== before.rows)
            failures.push(`picker: ${label} shows ${now.rows} property rows, TypeScript ${before.rows}`);
    }
    // The spelling is what changes: the same property is `appId`, `app_id` and (Rust, a declaration) `app_id`.
    if (!shownBy.typescript.names.some((n) => /[a-z][A-Z]/.test(n)))
        failures.push("picker: TypeScript shows no camelCase property name");
    if (!shownBy.python.names.some((n) => n.includes("_")))
        failures.push("picker: Python shows no snake_case property name");
    if (!shownBy.rust.names.some((n) => n.includes("fn "))) failures.push("picker: Rust shows no `fn` declaration");
    if (JSON.stringify(shownBy.typescript.names) === JSON.stringify(shownBy.python.names))
        failures.push("picker: Python spells the properties as TypeScript does");
    // Arrow keys: from the selected radio, Right moves to the next language and focuses it.
    await page.getByRole("radio", { name: "TypeScript" }).click();
    await page.getByRole("radio", { name: "TypeScript" }).focus();
    await page.keyboard.press("ArrowRight");
    if ((await snapshot()).lang !== "python") failures.push("picker: ArrowRight did not choose Python");
    if (!(await page.getByRole("radio", { name: "Python" }).evaluate((el) => el === document.activeElement)))
        failures.push("picker: ArrowRight did not move the focus");
    await page.keyboard.press("ArrowLeft");
    await page.keyboard.press("ArrowLeft");
    if ((await snapshot()).lang !== "rust")
        failures.push("picker: ArrowLeft twice from Python did not reach Rust (wrap)");
    // The choice is kept for the next page, and `?lang=` picks one and keeps it.
    await page.goto(`${base}/reference/std/workspace`, { waitUntil: "networkidle" });
    if ((await snapshot()).lang !== "rust") failures.push("picker: the choice was not kept on the next page");
    await page.goto(`${base}/reference/std/workspace?lang=python`, { waitUntil: "networkidle" });
    if ((await snapshot()).lang !== "python") failures.push("picker: ?lang=python did not pick Python");
    await context.close();
}

// 4. A member's anchor: the page opens at it, the row is the target and is in view, a method is open, and no language lacks the anchor.
{
    const context = await browser.newContext({ viewport: VIEWPORTS.desktop, reducedMotion: "reduce" });
    const page = await context.newPage();
    for (const lang of LANGS) {
        await context.addInitScript((value) => localStorage.setItem("gnoblin-language", value), lang);
        for (const hash of ["apply", "scale"]) {
            await page.goto(`${base}/reference/core/output#${hash}`, { waitUntil: "networkidle" });
            const found = await page.evaluate((id) => {
                const el = document.getElementById(id);
                if (!el) return "no such element";
                const r = el.getBoundingClientRect();
                const open = el instanceof HTMLDetailsElement ? el.open : true;
                return { target: el.matches(":target"), top: r.top, bottom: r.bottom, open, height: innerHeight };
            }, hash);
            const where = `/reference/core/output#${hash} [${lang}]`;
            if (typeof found === "string") failures.push(`${where}: ${found}`);
            else {
                if (!found.target) failures.push(`${where}: the element is not the :target`);
                if (found.top < 0 || found.top > found.height)
                    failures.push(`${where}: the anchor is at ${Math.round(found.top)}px, not in view`);
                if (!found.open) failures.push(`${where}: the method is closed`);
            }
        }
    }
    await context.close();
}

// 5. Contrast: the text of the reference pages is at least 4.5:1 against what lies under it (3:1 for large text), in both schemes and in
//    each language, including the method and the row that an anchor opens and tints.
{
    const CONTRAST_PAGES = [
        ...REFERENCE_PAGES,
        "/reference/core/output#scale",
        "/reference/core/client#event-" + firstEvent,
    ];
    for (const [width, langs] of [
        [1440, LANGS],
        [375, ["typescript"]],
    ]) {
        for (const scheme of ["light", "dark"]) {
            const context = await browser.newContext({
                viewport: { width, height: 900 },
                colorScheme: scheme,
                reducedMotion: "reduce",
            });
            const page = await context.newPage();
            for (const lang of langs) {
                await context.addInitScript((value) => localStorage.setItem("gnoblin-language", value), lang);
                for (const url of CONTRAST_PAGES) {
                    await page.goto(base + url, { waitUntil: "networkidle" });
                    const found = await page.evaluate(() => {
                        const parse = (css) => {
                            let m = /^rgba?\(([^)]+)\)$/.exec(css);
                            if (m) {
                                const p = m[1]
                                    .split(/[ ,/]+/)
                                    .filter(Boolean)
                                    .map(Number);
                                return { r: p[0], g: p[1], b: p[2], a: p[3] ?? 1 };
                            }
                            m = /^color\(srgb ([^)]+)\)$/.exec(css);
                            if (m) {
                                const p = m[1].split(/[ /]+/).filter(Boolean).map(Number);
                                return { r: p[0] * 255, g: p[1] * 255, b: p[2] * 255, a: p[3] ?? 1 };
                            }
                            return null;
                        };
                        const over = (top, under) => ({
                            r: top.r * top.a + under.r * (1 - top.a),
                            g: top.g * top.a + under.g * (1 - top.a),
                            b: top.b * top.a + under.b * (1 - top.a),
                            a: 1,
                        });
                        const lum = ({ r, g, b }) => {
                            const f = (v) => ((v /= 255) <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4);
                            return 0.2126 * f(r) + 0.7152 * f(g) + 0.0722 * f(b);
                        };
                        const background = (el) => {
                            const layers = [];
                            for (let e = el; e; e = e.parentElement) {
                                const c = parse(getComputedStyle(e).backgroundColor);
                                if (c && c.a > 0) layers.push(c);
                                if (c && c.a === 1) break;
                            }
                            let base = { r: 255, g: 255, b: 255, a: 1 };
                            const body = parse(getComputedStyle(document.body).backgroundColor);
                            if (body && body.a > 0) base = body;
                            for (const layer of layers.reverse()) base = over(layer, base);
                            return base;
                        };
                        const out = [];
                        const seen = new Set();
                        for (const el of document.querySelectorAll("main *, header *")) {
                            if (!el.getClientRects().length) continue;
                            const own = [...el.childNodes].filter((n) => n.nodeType === 3 && n.textContent.trim());
                            if (!own.length) continue;
                            const style = getComputedStyle(el);
                            if (style.visibility === "hidden" || Number(style.opacity) < 1) continue;
                            const fg0 = parse(style.color);
                            if (!fg0) continue;
                            const bg = background(el);
                            const fg = over(fg0, bg);
                            const ratio = (Math.max(lum(fg), lum(bg)) + 0.05) / (Math.min(lum(fg), lum(bg)) + 0.05);
                            const size = parseFloat(style.fontSize);
                            const large = size >= 24 || (size >= 18.66 && Number(style.fontWeight) >= 700);
                            if (ratio >= (large ? 3 : 4.5)) continue;
                            const key = `${style.color}|${Math.round(ratio * 100)}`;
                            if (seen.has(key)) continue;
                            seen.add(key);
                            out.push(
                                `"${own[0].textContent.trim().slice(0, 24)}" ${ratio.toFixed(2)}:1 (${style.color} on ${Math.round(bg.r)},${Math.round(bg.g)},${Math.round(bg.b)}) in ${el.tagName.toLowerCase()}.${String(el.getAttribute("class") ?? "").split(" ")[0]}`,
                            );
                        }
                        return out.slice(0, 4);
                    });
                    for (const f of found) failures.push(`contrast ${url} [${width}, ${scheme}, ${lang}] ${f}`);
                }
            }
            await context.close();
        }
    }
}

// 6. Search finds a member by each of its three spellings, as an "API reference" result listed above the pages, and shows its name in the
//    language that is picked.
{
    const context = await browser.newContext({ viewport: VIEWPORTS.desktop, reducedMotion: "reduce" });
    const page = await context.newPage();
    await page.goto(`${base}/reference/core/client`, { waitUntil: "networkidle" });
    const spellings = {};
    for (const lang of LANGS) {
        await page.evaluate((l) => (document.documentElement.dataset.lang = l), lang);
        spellings[lang] = await page.evaluate((id) => {
            const row = document.getElementById(id);
            const cell = [...row.querySelectorAll("th[scope=row] a")].find((a) => a.getClientRects().length);
            return cell.textContent.trim().replace(/^"|"$/g, "");
        }, `event-${firstEvent}`);
    }
    const target = `/reference/core/client#event-${firstEvent}`;
    for (const lang of LANGS) {
        const urls = await page.evaluate(async (q) => {
            const pagefind = await import("/pagefind/pagefind.js");
            const found = await pagefind.search(q, { filters: { section: "reference" } });
            return (await Promise.all(found.results.map((r) => r.data()))).map((d) => d.url);
        }, spellings[lang]);
        if (!urls.includes(target))
            failures.push(`search: "${spellings[lang]}" (${lang}) does not find ${target} (${urls.slice(0, 3)})`);
    }
    // The dialog: the API reference above the pages, the name in the picked language, and the footer says which.
    for (const lang of LANGS) {
        const dialogContext = await browser.newContext({ viewport: VIEWPORTS.desktop, reducedMotion: "reduce" });
        await dialogContext.addInitScript((value) => localStorage.setItem("gnoblin-language", value), lang);
        const p = await dialogContext.newPage();
        await p.goto(`${base}/docs`, { waitUntil: "networkidle" });
        await p.keyboard.press("Control+k");
        await p.getByRole("combobox", { name: "Search the docs" }).fill("focus");
        await p.locator('#search-results [role="option"]').first().waitFor({ timeout: 10000 });
        const dialog = await p.evaluate(() => {
            const labels = [...document.querySelectorAll("#search-results .section")].map((s) => s.textContent.trim());
            const rows = [...document.querySelectorAll("#search-results .api .rt")].map((el) => el.textContent.trim());
            const foot = document.querySelector("dialog.search .foot")?.textContent ?? "";
            const pages = document.querySelector("#search-results .group:not(.api)") !== null;
            const apiBeforePages = (() => {
                const all = [...document.querySelectorAll("#search-results .group")];
                const firstPage = all.findIndex((g) => !g.classList.contains("api"));
                return firstPage === -1 || all.slice(0, firstPage).every((g) => g.classList.contains("api"));
            })();
            return { labels, rows, foot, pages, apiBeforePages };
        });
        const label = { typescript: "TypeScript", python: "Python", rust: "Rust" }[lang];
        if (dialog.labels[0] !== "API reference")
            failures.push(`search dialog [${lang}]: the first section is "${dialog.labels[0]}"`);
        if (!dialog.apiBeforePages)
            failures.push(`search dialog [${lang}]: a page is listed above an API reference result`);
        if (!dialog.foot.includes(`Names in ${label}`))
            failures.push(`search dialog [${lang}]: the footer does not say "Names in ${label}"`);
        if (!dialog.rows.length) failures.push(`search dialog [${lang}]: no API reference row`);
        // Spelling: a member that is two words is camelCase in TypeScript and snake_case in the other two.
        const spelled = lang === "typescript" ? /\.[a-z]+[A-Z]/ : /\.[a-z]+_[a-z]/;
        if (!dialog.rows.some((row) => spelled.test(row)))
            failures.push(`search dialog [${lang}]: no row is spelled in ${label} (${dialog.rows.slice(0, 3)})`);
        await dialogContext.close();
    }
    await context.close();
}

// 7. The old roots of the per-language references redirect to /reference, and nothing on the site links to them.
{
    const page = await browser.newPage();
    for (const url of ["/reference/typescript/", "/reference/python/", "/reference/rust/", "/docs/reference"]) {
        await page.goto(base + url, { waitUntil: "networkidle" });
        const at = new URL(page.url());
        if (at.pathname !== "/reference") failures.push(`redirect: ${url} ends at ${at.pathname}, expected /reference`);
    }
    await page.close();
    const old = /href="\/reference\/(typescript|python|rust)(\/|")/;
    const walkHtml = (dir) =>
        readdirSync(dir).flatMap((name) => {
            const file = path.join(dir, name);
            if (statSync(file).isDirectory()) return name === "pagefind" ? [] : walkHtml(file);
            return name.endsWith(".html") ? [file] : [];
        });
    for (const file of walkHtml(path.join(site, "build"))) {
        const rel = path.relative(path.join(site, "build"), file);
        if (/^reference\/(typescript|python|rust)\//.test(rel)) continue;
        if (old.test(readFileSync(file, "utf8"))) failures.push(`${rel} links to an old per-language reference`);
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
