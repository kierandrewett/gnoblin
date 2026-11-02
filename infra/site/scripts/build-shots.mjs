// Writes the screenshots the site shows to static/shots/: each capture as WebP and PNG at 1x and 2x of the 720 px
// column (720 and 1440 px wide; smaller captures at half and full size, never enlarged), plus their sizes in
// src/lib/shots.json. The outputs are committed, so a site build needs neither the originals nor sharp.
//
// The originals are real captures of a running Gnoblin session (see the capture log in the design source). Re-run
// with `yarn shots` after replacing a capture. SHOTS_DIR points at the originals (default /var/tmp/shots).
import { mkdirSync, rmSync, writeFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import sharp from "sharp";

const site = fileURLToPath(new URL("..", import.meta.url));
const source = process.env.SHOTS_DIR ?? "/var/tmp/shots";
const out = path.join(site, "static/shots");

// Never add the control centre or calendar captures, or any *-network-sample capture: they show sample data.
const SHOTS = [
    "gnoblin-bingux-desktop-light",
    "gnoblin-bingux-desktop-dark",
    "gnoblin-bingux-search-light",
    "gnoblin-bingux-search-dark",
    "gnoblin-bingux-notification-light",
    "gnoblin-bingux-notification-dark",
    "bingux-files-light",
    "bingux-files-dark",
    "bingux-settings-light",
    "bingux-settings-dark",
];

rmSync(out, { recursive: true, force: true });
mkdirSync(out, { recursive: true });
const manifest = {};
for (const name of SHOTS) {
    const input = path.join(source, `${name}.png`);
    const meta = await sharp(input).metadata();
    const full = Math.min(1440, meta.width);
    const sizes = [Math.round(full / 2), full];
    manifest[name] = sizes.map((width) => ({ width, height: Math.round((meta.height * width) / meta.width) }));
    for (const width of sizes) {
        const resized = sharp(input).resize({ width });
        await resized
            .clone()
            .webp({ quality: 82, effort: 6 })
            .toFile(path.join(out, `${name}-${width}.webp`));
        await resized
            .clone()
            .png({ palette: true, quality: 90, compressionLevel: 9, effort: 10 })
            .toFile(path.join(out, `${name}-${width}.png`));
    }
    console.log(`${name}: ${sizes.join(", ")}`);
}
writeFileSync(path.join(site, "src/lib/shots.json"), JSON.stringify(manifest, null, 2) + "\n");
