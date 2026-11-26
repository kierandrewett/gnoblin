// Writes the API reference pages (TypeScript, Python and Rust, for the core and std libraries) into .generated/docs, where the
// site reads them beside the pages of docs/ (src/lib/server/docs.ts). `yarn build` runs it first.
//
// The pages come from the SDK generator, which is Rust: `cargo xtask docs-reference` in the repository root, from the schema
// catalog and the doc comments in the schema files. Nothing it writes is committed (the folder is ignored by Git).
//
//   node scripts/reference.mjs               always write the pages
//   node scripts/reference.mjs --if-missing  write them only when they are not there (yarn dev)
import { spawnSync } from "node:child_process";
import { existsSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const site = fileURLToPath(new URL("..", import.meta.url));
const root = path.resolve(site, "../..");
const out = path.join(site, ".generated/docs");

if (process.argv.includes("--if-missing") && existsSync(path.join(out, "reference/index.md"))) process.exit(0);

const run = spawnSync("cargo", ["xtask", "docs-reference", "--out", out], { cwd: root, stdio: "inherit" });
if (run.error) {
    console.error(
        `Cannot run cargo (${run.error.message}). The API reference is written by the SDK generator, which needs the Rust toolchain of the repository.`,
    );
    process.exit(1);
}
process.exit(run.status ?? 1);
