# Gnoblin Markdown rules

Use these rules for Gnoblin documentation, API references, examples, and
screenshots. This file is the source of truth for the Markdown review script
and Jev. Keep editorial rules here instead of duplicating them in code.

## Make the page easy to scan

- Give each paragraph one main idea. Start a new paragraph when the subject,
  explanation, or reader action changes.
- Break up long paragraphs deliberately. Several settings, options, caveats,
  or steps chained into one paragraph are a sign to use paragraphs, a list, or
  a table.
- Source-code line wrapping does not create a visible paragraph break. Use a
  blank line between paragraphs.
- Use headings to separate concepts and tasks, bullets for independent items,
  numbered lists for ordered steps, and tables to compare settings or values.
- Do not bury a long list of events, enum values, options, or actions in a
  sentence. Put the items in bullets or a table, and group them by purpose
  when that helps readers choose.
- Use Markdown structure when it makes relationships clearer. Do not turn
  every sentence into its own paragraph or add tables for a short, simple fact.
- Use inline code for short identifiers, setting names, key names, commands,
  and literal values. Use fenced code blocks for runnable examples and
  multi-line configuration.
- Format links as descriptive phrases so readers know what they will open.
- Give each page a clear job. Open with the task or answer, then move from a
  small working example to the details readers need to adapt it.
- Put related choices under short, task-based headings. Keep a section focused
  on one decision or step; split it when it starts answering a second question.
- Interleave explanation and examples where readers need them. Avoid a long
  block of code followed by a long block of unrelated explanation.
- Group long option lists by what the reader is trying to change. Use a short
  lead-in to say how to choose among groups.
- Use tables for compact comparisons whose cells scan as short values or
  phrases. If descriptions wrap into sentences, use grouped bullets instead;
  split a large table by topic rather than squeezing it into narrow columns.
  Keep table cells under about 24 prose words; move explanations into nearby
  bullets when a cell needs more space.
- Do not place a table and then explain its rows again in prose. Follow-up
  prose should explain interactions, limits, discovery steps, or choices that
  the table does not already state. If prose repeats three or more field
  definitions from one table, keep each definition in one place.

## Explain settings and accepted values

- For each documented option, state its type, accepted values, default, and
  effect when those details are known. Say when a change takes effect, such as
  immediately, on reload, or at the next login.
- Define API terms whose meaning or values are not clear from their name and
  context. Explain what the field, option, argument, event, or protocol term
  controls and what the reader can put there. Do not assume a generic noun
  explains itself.
- Distinguish closed choices from user-defined values. List every supported
  enum value; for open values, describe the format, constraints, and how to
  discover a valid value on the reader's system.
- Do not bury multiple option definitions in prose. Give each setting or
  protocol field its own table row or list item.
- Prefer a compact reference table with columns such as `Field`, `Type or
accepted values`, `Default`, and `Meaning or example`. Add an inline example
  beside the table or a short code block that uses several values together.
- When a value is an enum or an unfamiliar term, list the accepted choices
  and show a short example. For XCB and similar identifiers, explain what the
  values select and link to the authoritative reference when the full set is
  defined externally.
- For values that depend on the user's system, explain how to discover them.
  Examples include application IDs, window titles, keyboard layouts, and
  protocol globals.
- Link to primary, maintained references for external behavior. Link to the
  relevant section or value list, and explain which part the Gnoblin option
  uses.
- Give readers a next step when a concept has a larger vocabulary or external
  behavior. Link to the Gnoblin reference, implementation source, protocol
  XML/specification, or maintained upstream guide that defines the values.
  Prefer links to exact sections over a generic project homepage.
- Do not guess accepted values or defaults. Check the implementation or an
  authoritative source; mark uncertainty plainly when it cannot be confirmed.

## Keep API reference current and complete

- Document the current public API with complete examples. Show where
  configuration belongs and when it takes effect.
- Keep every supported public `gnoblin.*` Lua API represented in the Config
  API sidebar. Add or update its reference page in the same change that adds
  the API.
- For APIs with several fields or modes, document required and optional
  fields, accepted values, defaults, and a realistic example.
- Keep current user guides about the current API. Remove deprecated, legacy,
  old-syntax, and migration descriptions from current documentation. Historical
  records, if retained, must be clearly archived and kept out of current
  navigation and search.
- Verify names and behavior against the current source before publishing.
  Similar names do not guarantee similar behavior.

## Make examples useful

- Include examples for Gnoblin features that help readers complete a task.
  Cover built-in features such as the developer console as well as the
  compositor, configuration API, and shell integration.
- Keep examples runnable or label pseudocode clearly. Identify prerequisites,
  where code goes, and the action that applies or runs it.
- Prefer small, complete examples over fragments that omit required context.
  Explain only the details needed to adapt them.
- For APIs such as the frame renderer, include concise inline usage examples
  and language-neutral or JavaScript-like pseudocode when it clarifies the
  lifecycle or data flow.
- Make claims match the example and current implementation. Do not invent API
  names, behavior, output, or screenshots.

## Use screenshots to show the real result

- When a configuration example has a useful visual result, show that example
  running in Gnoblin or the application it configures.
- Capture the actual interface or application state. Do not create a fake UI,
  test report, or terminal screen to stand in for the feature.
- Do not use terminal text as a substitute for documentation prose or as
  narration for another screenshot. Readers can see the screen; explain the
  steps and relevant behavior in the page.
- A caption may identify context the image cannot show. Do not repeat visible
  labels or add decorative copy.
- Keep existing useful screenshots when editing docs. Replace them only when
  the new image is accurate, clearer, and shows the real feature.

## Keep prose direct and accurate

- Lead with the action or answer the reader needs. Prefer short, specific
  sentences and plain language.
- Remove repeated explanations, throat-clearing, generic introductions,
  conclusions that only restate the page, and prose that adds no information.
- When a table already lists fields, accepted values, defaults, or effects, do
  not restate those definitions in the prose that follows it. Follow-up prose
  should add context the table does not contain, such as interactions, limits,
  or when to use an option.
- Avoid prose blobs made of many inline-code terms separated by commas or
  semicolons. Turn the independent facts into headings, lists, or tables.
- When a paragraph contains six or more inline-code terms and at least 30
  prose words, split the definitions into bullets or short paragraphs and
  group related terms together.
- Do not explain obvious visible content in a screenshot or talk down to the
  reader. Explain decisions, steps, constraints, and behavior they cannot
  infer from the image alone.
- Preserve technical meaning while editing. Do not remove examples or change
  API behavior to make prose shorter.
- A paragraph should usually stay below 60 words. Split it when its subject or
  purpose changes, and use a list when it defines several independent facts.
  The Markdown validator flags paragraphs over 60 words, paragraphs with six
  or more inline-code terms and at least 30 prose words, and table cells over
  24 words for review; the counts are prompts to improve scanability, not a
  reason to break a coherent explanation into fragments.
