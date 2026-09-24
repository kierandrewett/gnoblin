import { writeFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import path from "node:path";

const root = fileURLToPath(new URL("../", import.meta.url));
const docs = path.join(root, "docs");
const output = path.join(docs, ".vitepress", "dist");
await writeFile(path.join(output, ".nojekyll"), "");
