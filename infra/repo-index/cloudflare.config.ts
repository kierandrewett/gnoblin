import { bindings, defineConfig, triggers } from "cf/config";
import * as entrypoint from "./src/index.ts" with { type: "cf-worker" };

export default defineConfig({
    accountId: "d6d18e6ddbe1f32180aa5f3bb96d4fc6",
    worker: {
        name: "repo-index",
        entrypoint,
        compatibilityDate: "2026-10-01",
        env: {
            REPO: bindings.r2({ name: "gnoblin-repo" }),
        },
        triggers: [triggers.fetch({ pattern: "repo.gnoblin.org/*", zone: "gnoblin.org" })],
    },
});
