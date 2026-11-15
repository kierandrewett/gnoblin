# Editor support

Get completion, argument hints and type checking for `gnoblin` in your editor.

The types are the library's own TypeScript. `gnoblinctl types init` writes that source next to your `init.ts`, with a
`tsconfig.json` that finds it.

Any editor that runs the TypeScript language service, such as VS Code, Neovim, Helix or Zed, then:

- completes every member of the `gnoblin` object and underlines a wrong one,
- shows the comment written above a class when you hover it,
- opens the file the shell runs when you go to the definition of an import.

## Set it up

1. Run `gnoblinctl types init`. It writes a `.gnoblin` directory and a `tsconfig.json` beside your `init.ts`.
   `gnoblinctl config init` runs it for you.
2. Open `init.ts`. Typing `gnoblin.workspaces.` now lists the members with their signatures, and hovering a call shows its
   description.
3. Check a file from the command line with `gnoblinctl check --types ~/.config/gnoblin/init.ts`. It runs a TypeScript
   compiler when one is installed.

## What is written where

```text
~/.config/gnoblin/
  init.ts
  tsconfig.json         yours: it extends .gnoblin/tsconfig.json
  .gnoblin/             generated: do not edit it, do not commit it
    tsconfig.json
    gnoblin.d.ts
    lib/gnoblin/        the library
```

- `import { Camera3D, Node } from "gnoblin/scene"` resolves to `.gnoblin/lib/gnoblin/scene.ts`, which re-exports the
  classes from `scene/camera.ts` and `scene/node.ts`. Every module name maps to the file of that name under
  `.gnoblin/lib/gnoblin`, and `"gnoblin"` is `index.ts`.
- A class extends the class it really extends: `Camera3D` is assignable to `Node`, and a `Client` to a `NativeObject`.
- `.gnoblin/gnoblin.d.ts` holds what is not source: the interfaces generated from the object schema (the modules
  `gnoblin/schema/...`) and the two globals, `gnoblin` and `console`.
- `tsconfig.json` is yours. It extends the generated one, so a release can change the compiler options and the paths
  without touching your file. Add your own `include`, `exclude` or options next to the `extends`. A `files` list of
  your own replaces the generated one, which names `gnoblin.d.ts`: keep `./.gnoblin/gnoblin.d.ts` in it.

`gnoblinctl types init` never replaces a `tsconfig.json` you already have. It rewrites `.gnoblin` every time, so run it
again after you update Gnoblin: the files match the release that wrote them. If your `tsconfig.json` does not extend
`.gnoblin/tsconfig.json`, the command says so, and the editor cannot find the library until it does.

## The same hints in the console

`gnoblinctl console` evaluates expressions in the running shell: see [Console and inspector](/js-console).
