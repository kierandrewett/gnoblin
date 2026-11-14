import { createRequire } from "node:module";

const require = createRequire(process.argv[1]);
const tseslint = require("typescript-eslint");

// Qt directives are removed only from the text passed to ESLint.
const qtDirectives = {
    processors: {
        javascript: {
            preprocess: (text) => [text.replace(/^\.(?:pragma|import)\b.*$/gm, (line) => " ".repeat(line.length))],
            postprocess: (messages) => messages.flat(),
        },
    },
};

export default [
    {
        files: ["**/*.js", "**/*.mjs", "**/*.cjs"],
        plugins: { qt: qtDirectives },
        processor: "qt/javascript",
        rules: {
            "constructor-super": "error",
            "for-direction": "error",
            "getter-return": "error",
            "no-async-promise-executor": "error",
            "no-constant-binary-expression": "error",
            "no-dupe-args": "error",
            "no-dupe-else-if": "error",
            "no-dupe-keys": "error",
            "no-duplicate-case": "error",
            "no-func-assign": "error",
            "no-self-assign": "error",
            "no-unreachable": "error",
            "valid-typeof": "error",
        },
    },
    ...tseslint.configs.recommended.map((config) => ({ ...config, files: ["**/*.ts"] })),
    {
        files: ["**/*.ts"],
        languageOptions: { parserOptions: { projectService: false, tsconfigRootDir: import.meta.dirname } },
        rules: {
            "@typescript-eslint/no-unused-vars": [
                "error",
                {
                    args: "after-used",
                    argsIgnorePattern: "^_",
                    caughtErrorsIgnorePattern: "^_",
                    destructuredArrayIgnorePattern: "^_",
                    varsIgnorePattern: "^_",
                },
            ],
            "@typescript-eslint/no-explicit-any": "error",
            "@typescript-eslint/no-non-null-assertion": "off",
            eqeqeq: ["error", "always", { null: "ignore" }],
            "no-shadow": "off",
            "no-var": "error",
            "prefer-const": "error",
        },
    },
    {
        // These APIs intentionally merge optional hooks into classes without creating instance fields.
        files: ["data/js/gnoblin/layout.ts", "data/js/gnoblin/ui.ts"],
        rules: { "@typescript-eslint/no-unsafe-declaration-merging": "off" },
    },
    {
        // These files are generated from the same schema; lint the handwritten API sources instead.
        files: [
            "crates/gnoblin-js-types/static/gnoblin-stock.d.ts",
            "crates/gnoblin-js-types/tests/golden/gnoblin.d.ts",
        ],
        rules: {
            "@typescript-eslint/no-empty-object-type": "off",
            "@typescript-eslint/no-unsafe-function-type": "off",
            "@typescript-eslint/no-unsafe-declaration-merging": "off",
            "@typescript-eslint/ban-ts-comment": "off",
        },
    },
    { files: ["**/*.cjs"], languageOptions: { sourceType: "commonjs" } },
];
