# Editor support

Get completion, argument hints and type checking for `gnoblin` in your editor.

Gnoblin publishes a `gnoblin.d.ts` declaration file and a `tsconfig.json` that finds it. Any editor that runs the TypeScript
language service, such as VS Code, Neovim, Helix or Zed, then completes every `defineConfig` key and every member of the
`gnoblin` object, and underlines a wrong one.

## Set it up

1. Run `gnoblinctl types init`. It writes `gnoblin.d.ts` and `tsconfig.json` beside your `init.ts`. `gnoblinctl config init`
   runs it for you.
2. Open `init.ts`. Typing `gnoblin.workspaces.` now lists the members with their signatures, and hovering a call shows its
   description.
3. Check a file from the command line with `gnoblinctl check --types ~/.config/gnoblin/init.ts`. It runs a TypeScript
   compiler when one is installed.

`gnoblinctl types init` never replaces a `tsconfig.json` you already have, and it refreshes `gnoblin.d.ts` every time. Run it
again after you update Gnoblin, because the file matches the release that wrote it.

## The same hints in the console

`gnoblinctl console` evaluates expressions in the running shell: see [Console and inspector](/js-console).
