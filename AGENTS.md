# Gnoblin agent and contributor guidance

Follow these rules for all work in this repository, together with the user's
current instructions and any more specific `AGENTS.md` files. The user's
explicit direction takes precedence. Keep this file focused on durable working
rules; put detailed procedures in `CONTRIBUTING.md` or beside the relevant code.

## Establish the task before changing files

- Read the relevant issue, nearby code, existing documentation, and applicable
  repository guidance before deciding how to implement a request.
- Inspect `git status`, the current branch, and diffs in the files and
  submodules you may touch. This repository may contain valuable in-progress
  work. Preserve it; do not reset, clean, overwrite, or stage unrelated work.
- Turn the request into a clear user-visible outcome and check the whole path
  needed to deliver it. Do not stop at a plan, isolated source edit, or partial
  fix when the request calls for implementation.
- Follow constraints and decisions already given in the conversation. Do not
  ask the user to repeat settled preferences. Ask only when missing information
  materially blocks a safe or correct decision; otherwise proceed with a
  reasonable, stated assumption.

## Product boundaries and implementation

- Gnoblin owns compositor and session services. Shell-specific presentation
  belongs to a shell integration or shell project. Keep these responsibilities
  separate and use their documented interfaces.
- Keep upstream dependencies at their pinned revisions. Put Gnoblin-owned
  behavior in Gnoblin's source and overlays; keep upstream patches narrow and
  review their exact diff before applying them.
- Trace behavior across the relevant configuration, protocol, compositor,
  shell, packaging, and startup boundaries. Change only the parts required by
  the requested outcome, but do not leave required integration incomplete.
- Keep compatibility and failure behavior explicit. Where a feature depends on
  system, protocol, or security guarantees, consult authoritative sources and
  retain safe behavior until the required guarantee is actually established.
- Respect the product's existing interaction model when improving UI. Fix the
  reported friction within the established model unless the user asks to
  replace it. Do not substitute cosmetic changes for working behavior.
- Prefer a small coherent implementation over a pile of special cases. Do not
  invent settings, public APIs, architecture, or abstractions without a real
  user need and evidence in the existing design.

## Public documentation

- `docs/` is the published product documentation. It is for users and external
  developers integrating with Gnoblin. Every page must help readers install,
  configure, use, troubleshoot, or integrate the product through a supported
  interface.
- Keep internal implementation rationale, design proposals, security analysis,
  test plans, rollout status, and release evidence out of the published site.
  Put those in source comments where useful, contributor material, or a
  repository-level design area.
- Start with the reader's task and give direct instructions. Explain only the
  concepts needed to complete that task. Keep prose, navigation, page titles,
  and URLs plain, consistent, and easy to scan.
- Document the real product and public API. Derive names and available options
  from the source of truth; do not invent configuration leaves, promise
  unsupported behavior, or use paths and titles that obscure what a page
  covers.
- Make examples complete, runnable, and relevant. Say where they go and what
  the user should observe. Keep the primary workflow in the README and link to
  focused pages for detail instead of scattering or duplicating instructions.
- Use screenshots to show the actual interface or outcome. Capture a clean,
  representative product state; screenshots should look like documentation,
  not smoke-test output. Do not put terminal text, status labels, or narration
  into an image to explain what the image shows. Do not add decorative copy to
  repeat visible content or to stand in for clear instructions. Keep
  comparisons with other projects factual and respectful.
- Before finishing documentation changes, check links, navigation, names, and
  examples against the current source and intended audience. Use the
  documentation build when verification is requested or needed for the change.

## Writing style

- Use plain, specific language. Prefer concrete nouns and verbs, short
  paragraphs, and headings that describe the content beneath them.
- Lead with the result or action the reader needs. Remove throat-clearing,
  generic introductions, repeated conclusions, marketing language, and filler
  that adds no information.
- Make claims proportionate to the evidence. Separate confirmed behavior from
  assumptions, proposals, and missing verification; do not make prose sound
  more certain or polished than the underlying work.
- Avoid presentation that calls attention to itself. Use the simplest layout
  that makes the information clear; do not add ornamental panels, oversized
  labels, decorative terminal blocks, or visual flourishes without a reader
  need.
- Before publishing, look for common signs of slop: screenshots staged like
  test reports; terminal output used to narrate screenshots; image captions
  that repeat visible text; verbose or generic explanations; invented API
  names; inconsistent page names or paths; and gratuitous criticism of other
  projects. Replace these with real product evidence and concise, accurate
  explanation.

## Verification and evidence

- Choose verification that matches the changed boundary and the requested
  outcome. Prefer the narrowest meaningful existing check, then broaden it
  when the change crosses more components. The testing guide describes what
  each check proves and which checks need a running session, real seat, or
  hardware.
- Distinguish source edits, successful builds, installed artifacts, active
  runtime behavior, and published deployments. Evidence at one level does not
  prove the next.
- For user-facing behavior, verify the actual path as far end-to-end as the
  environment permits. A fixture or successful compile alone does not prove a
  running desktop behaves correctly.
- Report what was changed and what was actually checked. State material gaps
  plainly; do not imply runtime, packaging, hardware, or publishing success
  without evidence.

## Release packaging

- Before changing RPMs, release workflows, or COPR targets, read
  `design/release-packaging.md` and verify the live COPR chroots, latest build
  IDs, Fedora versions, and installed package versions. Old release notes are
  context, not proof of current publication state.
- Keep the Fedora build matrix, COPR project chroots, and package install
  checks aligned. A source compile is not evidence that the RPM dependency
  metadata permits installation on that Fedora release.
- After packaging or publication work, update
  `design/release-packaging.md` with the target matrix, exact build IDs/NVRs,
  checks performed, and any remaining runtime or release gap.

## Scope, collaboration, and delivery

- Keep changes reviewable and limited to the request. Do not commit, push,
  publish, deploy, or change external systems unless the user has asked for it.
- When collaboration is requested, split only independent work, define file or
  responsibility ownership, and integrate the results against the shared
  worktree without reverting another contributor's changes.
- If a request includes research, establish the relevant facts from primary or
  authoritative sources before making product or compatibility claims.
- Do not leave actionable bugs or follow-up tasks only in chat. Use the issue
  process below when creating or changing tracked work.

## Issue tracking

Gnoblin uses Beads (`bd`) for the dependency-aware work queue and GitHub Issues
as the durable, visible issue list. Keep each actionable bug or task visible in
GitHub so it can be picked up from another machine or session.

- Before choosing work, run `./scripts/gnoblin-issues sync --pull-only`, then
  `bd ready`. Search with `bd search` before filing a duplicate.
- A pre-commit hook runs bidirectional Beads/GitHub sync. Install it once with
  `uv tool run --from pre-commit==4.6.2 pre-commit install`. If offline, skip
  only this hook explicitly with `SKIP=beads-github-sync git commit ...` and
  sync again before the next commit.
- To file from Beads, run `bd create "..."` and immediately publish the
  returned ID with `./scripts/gnoblin-issues push <id>`.
- To file directly in GitHub, use `gh issue create --repo kierandrewett/gnoblin`
  and import it with `./scripts/gnoblin-issues pull <issue-url-or-number>`.
- After changing a synced issue or closing it with `bd close`, publish that ID
  with `./scripts/gnoblin-issues push <id>`.
- Manual syncs should specify `--pull-only` or `--push-only`. The pre-commit
  hook runs bidirectionally with `--prefer-newer`; if that reports a conflict,
  resolve it deliberately before proceeding.
- Beads' dependency graph is stored separately from GitHub Issues. Use
  `bd dolt pull` before coordinating from another clone and `bd dolt push`
  after changes when sharing that graph through the configured GitHub remote.

See `.beads/README.md` for setup and command details.
