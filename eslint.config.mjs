import { createRequire } from "node:module";

const require = createRequire(process.argv[1]);
const tseslint = require("typescript-eslint");

export default [
    // A recorded declaration emit, compared and never edited (crates/gnoblin-std-impl/tests/ORACLE.md, section 5).
    { ignores: ["tests/ts-compat/old-surface/**"] },
    {
        files: ["**/*.js", "**/*.mjs", "**/*.cjs"],
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
        // A listener method is also a named subscription of the objects that take the listener. The interface that
        // declares those members from the listener class merges into the class, and has no members of its own.
        files: [
            "data/js/gnoblin-core/host/client.ts",
            "data/js/gnoblin-core/host/collections.ts",
            "data/js/gnoblin-core/host/gnoblin.ts",
            "data/js/gnoblin-core/host/output.ts",
            "data/js/gnoblin-core/session/calls.ts",
        ],
        rules: {
            "@typescript-eslint/no-empty-object-type": "off",
            "@typescript-eslint/no-unsafe-declaration-merging": "off",
        },
    },
    {
        // These APIs intentionally merge optional hooks into classes without creating instance fields.
        files: ["data/js/gnoblin-std/layout.ts", "data/js/gnoblin-std/ui/canvas.ts"],
        rules: { "@typescript-eslint/no-unsafe-declaration-merging": "off" },
    },
    { files: ["**/*.cjs"], languageOptions: { sourceType: "commonjs" } },
];
