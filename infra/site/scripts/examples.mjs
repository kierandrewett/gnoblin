// Checks the examples of every `::: tabs` group in docs/ in their own language, so a page cannot show a call the SDK does not have.
// `yarn build` runs it before the build. It runs `cargo xtask docs-examples` in the repository root: TypeScript with tsc and Python
// with pyright (both installed by `yarn install` in this folder), and Rust with `cargo check` against the SDK crates. A failure
// names the page and the line in it. `node scripts/examples.mjs --lang python` checks one language.
import { spawnSync } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(fileURLToPath(new URL("..", import.meta.url)), "../..");

const run = spawnSync("cargo", ["xtask", "docs-examples", ...process.argv.slice(2)], { cwd: root, stdio: "inherit" });
if (run.error) {
    console.error(
        `Cannot run cargo (${run.error.message}). The examples are checked against the SDK crates, which need the Rust toolchain of the repository.`,
    );
    process.exit(1);
}
process.exit(run.status ?? 1);
