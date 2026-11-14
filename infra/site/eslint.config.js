import { createRequire } from "node:module";

const require = createRequire(process.argv[1]);
const js = require("@eslint/js");
const svelte = require("eslint-plugin-svelte");
const globals = require("globals");
const ts = require("typescript-eslint");

export default ts.config(
    { ignores: ["build/", ".svelte-kit/", "node_modules/", "static/"] },
    js.configs.recommended,
    ...ts.configs.recommended,
    ...svelte.configs.recommended,
    {
        languageOptions: {
            globals: { ...globals.browser, ...globals.node },
            parserOptions: { projectService: false, tsconfigRootDir: import.meta.dirname },
        },
        rules: {
            // TypeScript checks these (yarn check).
            "no-undef": "off",
        },
    },
    {
        files: ["**/*.svelte", "**/*.svelte.ts"],
        languageOptions: { parserOptions: { parser: ts.parser, extraFileExtensions: [".svelte"] } },
        rules: {
            // The docs HTML is rendered at build time from the repository's own Markdown, not from user input.
            "svelte/no-at-html-tags": "off",
            // The site is served from the domain root with no base path, so plain hrefs are already resolved.
            "svelte/no-navigation-without-resolve": "off",
        },
    },
);
