# Gnoblin

## Register

product

## Users and purpose

Gnoblin is a compositor for people who use an external desktop shell. It keeps
window management and system integration in the compositor. Bingux and other
clients provide the desktop controls. These boundaries are described in
README.md and docs/bring-your-own-shell.md.

The developer console is for inspecting and changing the running compositor.
The requested reference is a Doom-style console at the top of the screen,
with JavaScript evaluation and inspection similar to developer tools.

## Design principles

- Keep the no-frills presentation described in README.md.
- Make keyboard input and results the main content of the console.
- Keep the console above application content and restore focus when it closes.
- Show errors, pending work and the current execution context clearly.
- Keep ordinary desktop controls in the external shell.

## Accessibility and interaction

Use the existing Shell text, focus and input components. Provide keyboard
operation, labelled controls, selectable output and visible focus states.
Respect the Shell animation and high-contrast settings.

## References and boundaries

Looking Glass supplies the existing compositor evaluation and modal-input
patterns. Its GNOME extension UI is outside this console. A terminal emulator
is also outside this console: input is JavaScript in the compositor process.
