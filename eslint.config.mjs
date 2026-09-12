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
    { files: ["**/*.cjs"], languageOptions: { sourceType: "commonjs" } },
];
