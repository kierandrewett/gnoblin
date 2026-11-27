# Product

## Register

product

## Users

- Shell authors who write a desktop shell for Gnoblin in TypeScript, Python or Rust. They read a page with an editor
  open, copy an example, change it and run it. They come back to the API reference many times a day.
- People who install Gnoblin and use it as their desktop session. They follow the install page for their distribution,
  then look up one configuration setting or one troubleshooting step at a time.

Both groups arrive with a specific question. The site succeeds when they find the answer, read it, and leave to act on
it, in as few steps as possible.

## Product Purpose

gnoblin.org is the documentation of Gnoblin: how to install it, configure it, write a shell for it, and use its
public APIs. Every page helps a reader install, configure, use, troubleshoot, or integrate through a supported
interface. Internal design notes, test plans and release evidence are not part of the site.

Success looks like this: a new reader gets from the home page to a running shell in their language without a dead end;
a returning reader finds a member of the API by search or the sidebar in a few seconds; no page scrolls sideways on a
phone.

## Brand Personality

Plain, exact, quiet. The voice of an experienced engineer who explains a thing once, correctly, and gets out of the
way. Confidence comes from accurate, complete, runnable examples, not from decoration or claims.

## Anti-references

- Marketing-style documentation: hero panels on content pages, large slogans, feature-card grids, gradients.
- Decoration that stands in for content: ornamental callouts, oversized labels, staged terminal screenshots, captions
  that repeat what an image already shows.
- Layouts that hide content: wide tables or code that scroll the whole page sideways, navigation that only works on
  desktop, tabs that forget the reader's language on every page.
- Gratuitous comparison with other projects.

## Design Principles

- Answer first: the task or the fact leads the page; context follows only as far as the reader needs it.
- One language choice, everywhere: a reader who picks TypeScript, Python or Rust sees that language in every example
  until they change it.
- Navigation is content: the sidebar, page order, titles and URLs say the same thing, and every page links to the
  next step.
- Show the real product: examples are complete and checked, screenshots show real Gnoblin states.
- Nothing breaks on a small screen: content never overflows the viewport; wide tables and code scroll inside their own
  box.
- Little prose, one reference: the written pages are install steps and guides only. Every class, member, event,
  record and error is in one API reference generated from the schema, in the site's own design, with TypeScript,
  Python and Rust spellings.
- One path for every language: no page is written for one language. Where setup differs by language, the page shows
  it in tabs.
- Public entries only: examples import `gnoblin-core` and `gnoblin-std` (and their Python and Rust equivalents), never
  generated or internal modules.

## Accessibility & Inclusion

WCAG 2.2 AA: text contrast at least 4.5:1 (3:1 for large text), visible focus on every control, full keyboard
navigation including the language tabs, search and the mobile menu, respect for `prefers-reduced-motion` and
`prefers-color-scheme`, and no meaning carried by colour alone.
