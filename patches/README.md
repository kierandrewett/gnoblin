# Gnoblin patches

Patches against the pinned upstream sources. Gnoblin-owned code belongs in the overlays under `src/`; a patch is only
the narrow hook that upstream needs, such as a Meson line, a call site or a fix. Each patch is a `git format-patch`
file with one `From: kierandrewett <kieran@drewett.dev>` header.

| Directory                   | Applied to                                                              |
| --------------------------- | ----------------------------------------------------------------------- |
| `mutter/`                   | `subprojects/mutter`, reset to its upstream tag first                   |
| `xdg-desktop-portal-gnome/` | `subprojects/xdg-desktop-portal-gnome`, reset to its upstream tag first |
| `portal-dependencies/`      | Meson subprojects that the portal bundles, by topic: `NN-libgxdp/`      |

## Layout

```text
patches/<project>/
  series                          the order patches apply in; the only source of that order
  <NN>-<topic>/<NNNN>-<slug>.patch
```

- `series` has one path per line, relative to `patches/<project>/`, with no blank lines and no comments. The build
  (CMake `gnoblin_patch_series`), the Nix package, `scripts/check-build-deps.py`, `scripts/list-tarball-sources.sh`,
  `scripts/manage-patches.py` and the tests read it. Nothing globs and sorts the directories.
- A topic directory names what its patches do. Its patches need not be neighbours in the series: a topic collects
  every patch of one subject, and the series interleaves topics where a later patch needs an earlier one's hunks.
- `<NN>` numbers the topics 01, 02, ... in the order each first appears in the series. `<NNNN>` numbers the patches of
  a topic 0001, 0002, ... in series order. Both are dense and unique, so a duplicate or a gap is an error.
- The slug (`<slug>` in the file name) is free text from `[A-Za-z0-9._+-]`. Tests find a patch by its slug through
  `scripts/patch_series.py` (`find`, `topic`, `series`), never by number, so renumbering does not break them.
  `scripts/manage-patches.py path PROJECT SLUG` prints the path of one patch.

The old layout (`10-tooling`, `99-typed-window-api`, `zz-monolithic-compositor`, ...) is mapped to this one in
`design/patches-layout-mapping.tsv`.

## Check

```sh
scripts/manage-patches.py check [PROJECT]
```

It fails when a `.patch` file is missing from its series, when the series names a file that does not exist, when an
entry is listed twice, when a number is not dense or does not follow the series, or when a patch lacks the author
header. `tests/check-fast.sh`, CI and every patch apply run it.

## Add a patch

1. Make the change in the pinned subproject checkout, on top of the patches it needs.
2. Export it to the end of the series. `export` and `export-worktree` append a new file to `series`:

    ```sh
    scripts/manage-patches.py export mutter HEAD \
      patches/mutter/<NN>-<topic>/<NNNN>-<slug>.patch
    ```

    Use the next number of an existing topic, or the next free topic number for a new topic.

3. Run `scripts/manage-patches.py check`. Then run `./build.sh --without-portal`, or the narrower test of the
   area, to prove that the whole series still applies.

## Reorder or insert a patch

Edit `series` with the current file names, then renumber:

```sh
scripts/manage-patches.py renumber mutter
scripts/manage-patches.py check
```

`renumber` renames the topic directories and files to the numbers that the series implies and rewrites `series`. It
changes no patch body. A patch still has to apply on the state the patches before it leave, so reordering past a patch
that touches the same hunks needs the later patch regenerated (see `CONTRIBUTING.md`).

To move a patch to another topic, `git mv` it into that topic directory (create `<NN>-<topic>/` for a new topic, with
any number), change its series line to match, and run `renumber`.
