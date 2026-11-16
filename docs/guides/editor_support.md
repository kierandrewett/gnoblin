# Editor support

Get completion, argument hints and type checking for `gnoblin` in your editor.

The types are the library's own TypeScript. `gnoblinctl types init` writes that source next to your `shell.ts`, with a
`tsconfig.json` that finds it.

Any editor that runs the TypeScript language service, such as VS Code, Neovim, Helix or Zed, then:

- completes every member of the `gnoblin` object and underlines a wrong one,
- shows the comment written above a class when you hover it,
- opens the file the shell runs when you go to the definition of an import.

## Set it up

1. Run `gnoblinctl types init`. It writes a `.gnoblin` directory and a `tsconfig.json` beside your `shell.ts`.
   `gnoblinctl config init` runs it for you.
2. Open `shell.ts`. Typing `gnoblin.workspaces.` now lists the members with their signatures, and hovering a call shows its
   description.
3. Check a file from the command line with `gnoblinctl check --types ~/.config/gnoblin/shell.ts`. It runs a TypeScript
   compiler when one is installed.

## What is written where

```text
~/.config/gnoblin/
  shell.ts
  tsconfig.json         yours: it extends .gnoblin/tsconfig.json
  .gnoblin/             generated: do not edit it, do not commit it
    tsconfig.json
    gnoblin.d.ts
    lib/                the library: gnoblin-core/ and gnoblin-std/
```

- `import { Camera3D, Node } from "gnoblin-std/scene"` resolves to `.gnoblin/lib/gnoblin-std/scene.ts`, which re-exports the
  classes from `scene/camera.ts` and `scene/node.ts`. Every module name maps to the file of that name under
  `.gnoblin/lib`, and `"gnoblin-core"` and `"gnoblin-std"` are the `index.ts` of their directories.
- A class extends the class it really extends: `Camera3D` is assignable to `Node`, and a `Client` to a `NativeObject`.
- `.gnoblin/gnoblin.d.ts` holds what is not source: the two globals, `gnoblin` and `console`. `gnoblin` is the
  library's `Gnoblin` class, so its members are the ones `gnoblin-core/host/gnoblin` implements.
- `tsconfig.json` is yours. It extends the generated one, so a release can change the compiler options and the paths
  without touching your file. Add your own `include`, `exclude` or options next to the `extends`. A `files` list of
  your own replaces the generated one, which names `gnoblin.d.ts`: keep `./.gnoblin/gnoblin.d.ts` in it.

`gnoblinctl types init` never replaces a `tsconfig.json` you already have. It rewrites `.gnoblin` every time, so run it
again after you update Gnoblin: the files match the release that wrote them. If your `tsconfig.json` does not extend
`.gnoblin/tsconfig.json`, the command says so, and the editor cannot find the library until it does.

## The same hints in the console

`gnoblinctl console` evaluates expressions in the running shell: see [Console and inspector](/js-console).
