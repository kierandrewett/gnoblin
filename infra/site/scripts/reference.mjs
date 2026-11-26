// Writes the API reference of the three SDKs into .generated/api/<language>, where scripts/postbuild.mjs copies it to
// build/reference/<language> (the site serves it at /reference/typescript/, /reference/python/ and /reference/rust/). `yarn build`
// runs it first.
//
// Each language gets the reference tool of its own ecosystem, run over the SDK the language installs, so nothing here describes a
// signature of its own:
//
//   typescript  TypeDoc over the declarations an install gives an editor (gnoblin-core and gnoblin-std)
//   python      Sphinx with sphinx-autoapi over the stub package an install gives an editor; the stubs are read, never imported
//   rust        rustdoc (`cargo doc`) over gnoblin-core, gnoblin-std and gnoblin-macros
//
// The TypeScript and Python tools read what `cargo xtask docs-reference` writes into .generated/sdk. The descriptions in all three
// are the comments of the schema files, which the SDK generator (crates/gnoblin-sdkgen) writes as doc comments.
// Nothing it writes is committed (.generated/ is ignored by Git).
//
//   node scripts/reference.mjs                        write all three
//   node scripts/reference.mjs --only python,rust     write only these languages
//
// Python: a virtual environment in .generated/venv holds the pinned packages of reference/python/requirements.txt. PYTHON names the
// interpreter that makes it (default python3); with uv on the PATH the environment is made and filled with uv.
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { cpSync, existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const site = fileURLToPath(new URL("..", import.meta.url));
const root = path.resolve(site, "../..");
const generated = path.join(site, ".generated");
const sdk = path.join(generated, "sdk");
const api = path.join(generated, "api");
const reference = path.join(site, "reference");

const LANGUAGES = ["typescript", "python", "rust"];
const wanted = process.argv.includes("--only")
    ? (process.argv[process.argv.indexOf("--only") + 1] ?? "").split(",")
    : LANGUAGES;
for (const language of wanted) {
    if (!LANGUAGES.includes(language)) fail(`--only takes ${LANGUAGES.join(", ")}, not ${language}`);
}

function fail(message) {
    console.error(`reference: ${message}`);
    process.exit(1);
}

/** Runs a command and stops the build when it fails. */
function run(command, args, options = {}) {
    const result = spawnSync(command, args, { stdio: "inherit", ...options });
    if (result.error) {
        fail(`cannot run ${command} (${result.error.message}). ${options.hint ?? ""}`.trim());
    }
    if (result.status !== 0) fail(`${command} ${args.join(" ")} failed (exit ${result.status ?? result.signal})`);
}

function has(command) {
    return spawnSync(command, ["--version"], { stdio: "ignore" }).status === 0;
}

const timings = [];
function step(name, work) {
    const start = performance.now();
    work();
    timings.push(`${name} ${((performance.now() - start) / 1000).toFixed(1)}s`);
}

function clear(dir) {
    rmSync(dir, { recursive: true, force: true });
    mkdirSync(dir, { recursive: true });
}

// The declarations an install gives an editor: the TypeScript library and the Python stub package, with the schema's descriptions.
if (wanted.includes("typescript") || wanted.includes("python")) {
    step("declarations", () =>
        run("cargo", ["xtask", "docs-reference", "--out", sdk], {
            cwd: root,
            hint: "The declarations are written by the SDK generator, which needs the Rust toolchain of the repository.",
        }),
    );
}

if (wanted.includes("typescript")) {
    step("typescript", () => {
        clear(path.join(api, "typescript"));
        run(path.join(site, "node_modules/.bin/typedoc"), ["--options", path.join(reference, "typedoc.json")], {
            cwd: site,
            hint: "Run `yarn install` in infra/site.",
        });
    });
}

if (wanted.includes("python")) {
    step("python", () => {
        const venv = path.join(generated, "venv");
        const requirements = path.join(reference, "python/requirements.txt");
        const stamp = path.join(venv, ".requirements");
        const hash = createHash("sha256").update(readFileSync(requirements)).digest("hex");
        const bin = path.join(venv, "bin");
        if (!existsSync(stamp) || readFileSync(stamp, "utf8") !== hash) {
            rmSync(venv, { recursive: true, force: true });
            const hint = "The Python reference needs Python 3.11 or later (set PYTHON to choose it).";
            if (has("uv")) {
                run("uv", ["venv", "--quiet", "--python", process.env.PYTHON ?? "3", venv], { hint });
                run("uv", ["pip", "install", "--quiet", "--python", path.join(bin, "python"), "-r", requirements]);
            } else {
                run(process.env.PYTHON ?? "python3", ["-m", "venv", venv], { hint });
                run(path.join(bin, "python"), ["-m", "pip", "install", "--quiet", "-r", requirements]);
            }
            writeFileSync(stamp, hash);
        }
        const out = path.join(api, "python");
        clear(out);
        run(
            path.join(bin, "python"),
            [
                "-m",
                "sphinx",
                "--builder",
                "html",
                "--quiet",
                "--fail-on-warning",
                "--jobs",
                "auto",
                "-d",
                path.join(generated, "sphinx-doctrees"),
                path.join(reference, "python"),
                out,
            ],
            { env: { ...process.env, GNOBLIN_PYTHON_STUBS: path.join(sdk, "share/gnoblin/types/python") } },
        );
    });
}

if (wanted.includes("rust")) {
    step("rust", () => {
        // Cargo writes the pages into its target directory, which a CARGO_TARGET_DIR moves.
        const metadata = spawnSync("cargo", ["metadata", "--format-version", "1", "--no-deps"], {
            cwd: root,
            encoding: "utf8",
            maxBuffer: 256 * 1024 * 1024,
        });
        if (metadata.status !== 0) fail(`cargo metadata failed: ${metadata.stderr ?? metadata.error}`);
        const docs = path.join(JSON.parse(metadata.stdout).target_directory, "doc");
        // The folder holds only these crates' pages: a page of a crate documented earlier would be served too.
        rmSync(docs, { recursive: true, force: true });
        const flags = ["-D", "warnings", "--html-in-header", path.join(reference, "rust/header.html")];
        run(
            "cargo",
            [
                "doc",
                "--no-deps",
                "--package",
                "gnoblin-core",
                "--package",
                "gnoblin-std",
                "--package",
                "gnoblin-macros",
            ],
            {
                cwd: root,
                env: { ...process.env, RUSTDOCFLAGS: [process.env.RUSTDOCFLAGS, ...flags].filter(Boolean).join(" ") },
                hint: "rustdoc is a component of the Rust toolchain of the repository.",
            },
        );
        const out = path.join(api, "rust");
        clear(out);
        cpSync(docs, out, { recursive: true });
        // rustdoc writes a page for each crate and none for the folder. The core is where a reader starts: it has the entry macro, the
        // object model and the pointer to the std library.
        writeFileSync(
            path.join(out, "index.html"),
            `<!doctype html>
<html lang="en-GB">
<head>
<meta charset="utf-8">
<title>Gnoblin Rust SDK</title>
<meta http-equiv="refresh" content="0; url=gnoblin_core/index.html">
<link rel="canonical" href="gnoblin_core/index.html">
</head>
<body><p>The Rust SDK: <a href="gnoblin_core/index.html">gnoblin_core</a>, <a href="gnoblin_std/index.html">gnoblin_std</a> and <a href="gnoblin_macros/index.html">gnoblin_macros</a>.</p></body>
</html>
`,
        );
    });
}

if (timings.length) console.log(`reference: ${timings.join(", ")}`);
