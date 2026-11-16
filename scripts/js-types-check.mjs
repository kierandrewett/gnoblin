// Proves what `gnoblinctl types init` writes, as an editor and `tsc` read it. Run by scripts/js-types-check.sh.
//
//   node scripts/js-types-check.mjs CONFIG_DIR TSC_OUTPUT TYPESCRIPT_JS
//
// CONFIG_DIR is a configuration directory `types init` wrote, with the programs of tests/js-types copied into it.
// TSC_OUTPUT is what `tsc -p CONFIG_DIR/tsconfig.json --pretty false` printed for it. TYPESCRIPT_JS is the compiler
// API (typescript.js), the engine behind an editor's language server, which answers the questions an editor asks:
//
// 1. The diagnostics are exactly the ones `// expect: TSxxxx` names in bad/, one line below the marker: the programs
//    that must compile do, and the wrong ones fail with the right error and no other.
// 2. `gnoblin`, `gnoblin/scene`, `gnoblin/extensions/xr`, ... resolve to the real source files under .gnoblin/lib.
// 3. Go-to-definition on an imported name opens the file that defines it.
// 4. A class extends the real class it names: `Camera3D` the one `Node`, `XRCamera3D` that `Camera3D`.
// 5. Hover shows the doc comment of the real source.

import { createRequire } from "node:module";
import fs from "node:fs";
import path from "node:path";

const [configArgument, tscOutput, typescriptPath] = process.argv.slice(2);
if (!configArgument || !tscOutput || !typescriptPath) {
    console.error("usage: js-types-check.mjs CONFIG_DIR TSC_OUTPUT TYPESCRIPT_JS");
    process.exit(2);
}
const configDir = fs.realpathSync(configArgument);
const ts = createRequire(import.meta.url)(typescriptPath);
const library = path.join(configDir, ".gnoblin", "lib", "gnoblin");

const failures = [];
const check = (condition, message) => {
    if (condition) console.log(`ok   ${message}`);
    else {
        console.log(`FAIL ${message}`);
        failures.push(message);
    }
};

// -- 1. the diagnostics ---------------------------------------------------------------------------------------------

const expected = new Map(); // "file:line" -> the codes allowed there
for (const file of fs.readdirSync(path.join(configDir, "bad"))) {
    const lines = fs.readFileSync(path.join(configDir, "bad", file), "utf8").split("\n");
    lines.forEach((line, index) => {
        const marker = /^\/\/ expect: (TS\d+(?:\|TS\d+)*)$/.exec(line);
        if (marker) expected.set(`bad/${file}:${index + 2}`, marker[1].split("|"));
    });
}
const actual = new Map(); // "file:line" -> the codes reported there
const unexpected = [];
for (const line of fs.readFileSync(tscOutput, "utf8").split("\n")) {
    const diagnostic = /^(.+?)\((\d+),\d+\): error (TS\d+): (.*)$/.exec(line);
    if (!diagnostic) continue;
    const where = `${path.relative(configDir, path.resolve(configDir, diagnostic[1]))}:${diagnostic[2]}`;
    if (expected.has(where)) actual.set(where, [...(actual.get(where) ?? []), diagnostic[3]]);
    else unexpected.push(`${where} ${diagnostic[3]} ${diagnostic[4]}`);
}
check(
    unexpected.length === 0,
    `no error outside the lines the wrong programs mark (${unexpected.join("; ") || "none"})`,
);
check(expected.size >= 5, `the wrong programs mark ${expected.size} lines`);
for (const [where, codes] of expected) {
    const reported = actual.get(where) ?? [];
    check(
        reported.length > 0 && reported.every((code) => codes.includes(code)),
        `${where} fails with ${codes.join(" or ")} (reported: ${reported.join(", ") || "nothing"})`,
    );
}

// -- the language service over the same configuration ---------------------------------------------------------------

const configPath = path.join(configDir, "tsconfig.json");
const parsed = ts.getParsedCommandLineOfConfigFile(
    configPath,
    {},
    {
        ...ts.sys,
        onUnRecoverableConfigFileDiagnostic: (diagnostic) => {
            throw new Error(ts.flattenDiagnosticMessageText(diagnostic.messageText, "\n"));
        },
    },
);
const host = {
    ...ts.sys,
    useCaseSensitiveFileNames: () => ts.sys.useCaseSensitiveFileNames,
    getScriptFileNames: () => parsed.fileNames,
    getScriptVersion: () => "1",
    getScriptSnapshot: (file) =>
        ts.sys.fileExists(file) ? ts.ScriptSnapshot.fromString(ts.sys.readFile(file) ?? "") : undefined,
    getCurrentDirectory: () => configDir,
    getCompilationSettings: () => parsed.options,
    getDefaultLibFileName: (options) => ts.getDefaultLibFilePath(options),
};
const service = ts.createLanguageService(host, ts.createDocumentRegistry());
const program = service.getProgram();
const checker = program.getTypeChecker();
const real = (file) => fs.realpathSync(file);
const inLibrary = (file, tail) => real(file) === real(path.join(library, tail));

// -- 2. resolution --------------------------------------------------------------------------------------------------

const importer = path.join(configDir, "scene.ts");
for (const [name, tail] of [
    ["gnoblin", "index.ts"],
    ["gnoblin/scene", "scene.ts"],
    ["gnoblin/scene/node", "scene/node.ts"],
    ["gnoblin/extensions/xr", "extensions/xr/index.ts"],
    ["gnoblin/ui", "ui.ts"],
    ["gnoblin/style", "style.ts"],
    ["gnoblin/input", "input.ts"],
    ["gnoblin/host/workspace", "host/workspace.ts"],
]) {
    const resolved = ts.resolveModuleName(name, importer, parsed.options, ts.sys).resolvedModule;
    check(
        resolved !== undefined && inLibrary(resolved.resolvedFileName, tail),
        `"${name}" resolves to .gnoblin/lib/gnoblin/${tail} (${resolved?.resolvedFileName ?? "unresolved"})`,
    );
}

// -- 3. go-to-definition --------------------------------------------------------------------------------------------

/** The file and the declared name the definition of `word` (first found after `after`) in `fixture` opens. */
function definitionOf(fixture, word, after = "") {
    const file = path.join(configDir, fixture);
    const text = fs.readFileSync(file, "utf8");
    const position = text.indexOf(word, after === "" ? 0 : text.indexOf(after));
    const found = service.getDefinitionAtPosition(file, position) ?? [];
    return found.map((definition) => ({ file: definition.fileName, name: definition.name }));
}
for (const [fixture, word, after, tail] of [
    ["scene.ts", "Camera3D", "new Camera3D(", "scene/camera.ts"],
    ["scene.ts", "Node", ": Node = camera", "scene/node.ts"],
    ["scene.ts", "XRCamera3D", "new XRCamera3D(", "extensions/xr/camera.ts"],
    ["host.ts", "Workspace", "new Workspace(", "host/workspace.ts"],
    ["host.ts", "Client", "(client: Client)", "host/client.ts"],
    ["modules.ts", "Style", "{ Box,", "style/style.ts"],
]) {
    const [first] = definitionOf(fixture, word, after);
    check(
        first !== undefined && inLibrary(first.file, tail),
        `go-to-definition of ${word} in ${fixture} opens ${tail} (${first?.file ?? "nothing"})`,
    );
}
const [schema] = definitionOf("modules.ts", "Config", "= Config;");
check(
    schema !== undefined && real(schema.file) === real(path.join(configDir, ".gnoblin", "gnoblin.d.ts")),
    `go-to-definition of the schema interface Config opens .gnoblin/gnoblin.d.ts (${schema?.file ?? "nothing"})`,
);

// -- 4. inheritance -------------------------------------------------------------------------------------------------

/** The class symbol a fixture imports as `name`. */
function importedClass(fixture, name) {
    const source = program.getSourceFile(path.join(configDir, fixture));
    const exported = checker.getSymbolsInScope(source, ts.SymbolFlags.Alias).find((symbol) => symbol.name === name);
    return checker.getAliasedSymbol(exported);
}
/** Where the class `symbol` is declared, and the class it extends. */
function heritageOf(symbol) {
    const declared = checker.getDeclaredTypeOfSymbol(symbol);
    const [base] = checker.getBaseTypes(declared);
    return {
        file: symbol.declarations[0].getSourceFile().fileName,
        base: base?.symbol,
    };
}
const camera = heritageOf(importedClass("scene.ts", "Camera3D"));
check(
    camera.base?.name === "Node" && inLibrary(camera.base.declarations[0].getSourceFile().fileName, "scene/node.ts"),
    "Camera3D extends the Node declared in scene/node.ts",
);
const xrCamera = heritageOf(importedClass("scene.ts", "XRCamera3D"));
check(
    xrCamera.base?.name === "Camera3D" &&
        inLibrary(xrCamera.base.declarations[0].getSourceFile().fileName, "scene/camera.ts"),
    "XRCamera3D (gnoblin/extensions/xr) extends the Camera3D declared in scene/camera.ts",
);
const client = heritageOf(importedClass("host.ts", "Client"));
check(
    client.base?.name === "NativeObject" &&
        inLibrary(client.base.declarations[0].getSourceFile().fileName, "native_object.ts"),
    "Client extends the NativeObject declared in native_object.ts",
);

// -- 5. hover -------------------------------------------------------------------------------------------------------

const cameraSource = fs.readFileSync(path.join(library, "scene", "camera.ts"), "utf8");
const comment = /\/\*\* (.+?) \*\/\nexport class Camera3D/.exec(cameraSource)?.[1];
check(comment !== undefined, "the real source documents Camera3D with a one-line comment");
const sceneFile = path.join(configDir, "scene.ts");
const sceneText = fs.readFileSync(sceneFile, "utf8");
const hover = service.getQuickInfoAtPosition(sceneFile, sceneText.indexOf("new Camera3D(") + 4);
const documentation = hover ? ts.displayPartsToString(hover.documentation) : "";
check(
    comment !== undefined && documentation.includes(comment),
    `hover on Camera3D shows the source's comment "${comment}" (${documentation || "nothing"})`,
);

if (failures.length > 0) {
    console.error(`\n${failures.length} check(s) failed`);
    process.exit(1);
}
console.log("\nthe editor files hold: real sources, real inheritance, real definitions");
