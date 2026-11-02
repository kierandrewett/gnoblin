import js from "@eslint/js";
import svelte from "eslint-plugin-svelte";
import globals from "globals";
import ts from "typescript-eslint";

export default ts.config(
    { ignores: ["build/", ".svelte-kit/", "node_modules/", "static/"] },
    js.configs.recommended,
    ...ts.configs.recommended,
    ...svelte.configs.recommended,
    {
        languageOptions: { globals: { ...globals.browser, ...globals.node } },
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
