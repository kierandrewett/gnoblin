// See https://svelte.dev/docs/kit/types#app.d.ts
declare global {
    /** Commit this site was built from (GITHUB_SHA or `git rev-parse HEAD`), set in vite.config.ts. */
    const __BUILD_SHA__: string;
    /** ISO time of the build, set in vite.config.ts. */
    const __BUILT_AT__: string;
    /** Absolute path of the repository's docs folder, set in vite.config.ts. */
    const __DOCS_DIR__: string;
}

export {};
