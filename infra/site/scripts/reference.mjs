// Writes the machine-readable reference of the object model to .generated/reference.json, which the site's reference pages are built
// from (src/lib/server/reference.ts). `yarn build` runs it first.
//
// The file is the `reference` target of the SDK generator (crates/gnoblin-sdkgen): every class, property, method, signal, enum, record and
// callback of gnoblin-core and gnoblin-std with its description, how each language spells it and the line each SDK file declares for it.
// Nothing in the site spells a member itself, and nothing it writes is committed (.generated/ is ignored by Git).
//
//   node scripts/reference.mjs                         run the generator (needs the Rust toolchain of the repository)
//   node scripts/reference.mjs --from path/to/reference.json
//                                                      copy a file that was generated elsewhere, for a machine that cannot build
//                                                      the SDK crates (the file must come from the same commit)
import { spawnSync } from "node:child_process";
import { copyFileSync, mkdirSync, readFileSync, rmSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const site = fileURLToPath(new URL("..", import.meta.url));
const root = path.resolve(site, "../..");
const generated = path.join(site, ".generated");
const out = path.join(generated, "reference.json");

function fail(message) {
    console.error(`reference: ${message}`);
    process.exit(1);
}

const from = process.argv.includes("--from") ? process.argv[process.argv.indexOf("--from") + 1] : undefined;
if (process.argv.includes("--from") && !from) fail("--from takes the path of a reference.json");

mkdirSync(generated, { recursive: true });
const start = performance.now();
if (from) {
    copyFileSync(path.resolve(from), out);
} else {
    const sdk = path.join(generated, "reference-sdk");
    rmSync(sdk, { recursive: true, force: true });
    const result = spawnSync("cargo", ["xtask", "gen-sdks", "--target", "reference", "--out", sdk], {
        cwd: root,
        stdio: "inherit",
    });
    if (result.error)
        fail(`cannot run cargo (${result.error.message}). The generator needs the Rust toolchain of the repository.`);
    if (result.status !== 0) fail(`cargo xtask gen-sdks failed (exit ${result.status ?? result.signal})`);
    copyFileSync(path.join(sdk, "reference/reference.json"), out);
    rmSync(sdk, { recursive: true, force: true });
}

const model = JSON.parse(readFileSync(out, "utf8"));
if (model.format !== 1) fail(`reference.json has format ${model.format}; the site reads format 1`);
console.log(
    `reference: ${path.relative(root, out)} (${model.namespaces.map((n) => `${n.name} ${n.counts.objects} classes`).join(", ")}) in ${((performance.now() - start) / 1000).toFixed(1)}s`,
);
