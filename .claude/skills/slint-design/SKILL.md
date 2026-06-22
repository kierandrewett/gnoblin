---
name: slint-design
description: Make Slint UIs look professionally designed. Web/product design best practices — spacing scale, vertical rhythm, type hierarchy, component proportions, breathing room, AND motion/animation (durations, easing, enter/exit, hover feedback) — adapted to Slint's layout + animate primitives. Use when building OR reviewing the look or feel of any .slint component — cards, menus, popups, panels, buttons, lists, transitions.
---

# Slint UI design

Slint gives you layout primitives but no taste. Most "ugly" Slint comes from three
mistakes: **magic-number spacing**, **no vertical rhythm**, and **cramped edges**.
Fix those and it looks designed. Apply this whenever you write or review a `.slint`.

For desktop shell chrome, a component is not visually done until a screenshot has
been reviewed. Static tests prove it renders; they do not prove it looks good.
Run the repo review loop before claiming visual polish:

```sh
scripts/design-review.sh /tmp/gnoblin-notif-cc-history-expanded.png
```

If the external score is below 7/10, keep iterating.

## 1. Spacing scale — never use arbitrary px

All spacing comes from a 4px scale. Pick from it; never invent `13px`, `7px`, `22px`.

| token | px | use for |
|---|---|---|
| xs | 4  | tight pairs (label↔value, icon↔text in a chip) |
| sm | 8  | between buttons, list-row inset, chip padding |
| md | 12 | icon↔text in a row, inter-group spacing |
| lg | 16 | **default container padding** (cards, popouts, rows) |
| xl | 24 | section separation, generous card padding |
| 2xl| 32 | page-level gutters |

In Slint use `HorizontalLayout`/`VerticalLayout` **`padding`** (inset from the
container edge) + **`spacing`** (gap between children). Reach for these before
absolute `x:`/`y:` — layouts give you rhythm for free. Prefer a `Tokens`/`Spacing`
global so the scale is named, not retyped.

## 2. Vertical rhythm & proximity

- **Container padding ≥ inter-child spacing.** A card with `padding: 16px` should
  have *more* room to its edges than between its rows. Cramped edges = the #1 tell.
- **Proximity = meaning.** Related items sit closer (title↔body = `xs`/4px), groups
  sit further (body↔action-buttons = `lg`/16px). Equal gaps everywhere read as a
  wall of text.
- **Equal padding all around** unless you have a reason. Don't pad 14 left / 8 top.
- **Let it breathe.** When unsure, add space. Dense ≠ professional.

## 3. Type hierarchy

Three sizes max per component; differentiate with **size + weight + opacity**, not
size alone.

- Title/summary: 15px, weight 600–700, full-opacity foreground.
- Body: 13–14px, weight 400.
- Secondary/caption: 12–13px at **~65% opacity** (e.g. `#ffffffa6` on dark) — dimming
  is what makes hierarchy read, not shrinking.
- Line spacing: give multi-line text a `spacing`/line-height ~1.4×; never set text
  flush against the next element.

## 4. Component proportions

- **Interactive targets ≥ 36px** tall (38–40 is comfortable). A button that's
  `height` ≈ its font-size looks cramped; pad it: ~10px vertical, 16px horizontal.
- **Buttons:** radius `8px`, a visible `has-hover` background step, centered label,
  consistent height across a row. Two buttons in a row → equal width, `sm`/8px gap.
- **Rows/list items:** `lg` horizontal inset, `36–44px` tall, hover background.
- **Radii nest:** inner radius < outer (a button inside a `14px` card uses `8px`).
  Same-radius nesting looks off.

## 5. Colour & surface

- One elevated surface colour, generous contrast for text, dimmed secondary text.
- Accents (urgency, selection) are a **thin** edge or a saturated fill — not a
  whole-card recolour.
- Shadows: soft + offset down (`drop-shadow-blur: 20–28px`, `offset-y: 6–8px`,
  low-alpha black). One shadow, not many.

## 5a. Icons — use a system, not a pile of SVGs

Icons fail fast in shell chrome because they are tiny, repeated, and always in
the user's peripheral vision.

- Pick one icon family per cluster. Do not mix filled GNOME, outline Lucide,
  custom stroke SVGs, and branded raster icons inside quick settings.
- Match optical weight, not just viewport size. A 24px SVG with thin 1.5px
  strokes can look weaker than a 16px symbolic icon with 2px strokes.
- Use recognized metaphors only. If an icon needs explanation and the tile has
  room for text, prefer a clearer text label or a more universal icon.
- Avoid icon-only controls unless space is genuinely constrained and the icon is
  conventional. If the action is not obvious, add a tooltip and visible state.
- State must not rely on a tiny shade change. Active/inactive needs at least two
  cues: surface colour plus icon/text contrast, check/indicator, or clear label.
- For topbar/menu-bar glyphs, prefer monochrome/template-style icons and keep
  them optically centered. They should feel like one set at a glance.

## 6. Motion & animation

Motion is design. Static UI feels cheap; *over*-animated UI feels amateur. The rule:
**animate state changes to explain them, then get out of the way.** Fast, purposeful,
mostly opacity + transform.

**Durations (fast — UI is not a film):**
- Hover / press / micro-feedback: **80–150ms**.
- State changes, toggles, small moves: **150–250ms**.
- Overlay enter/exit (menus, popouts, dialogs): **200–300ms**.
- Nothing over ~400ms for UI — longer reads as sluggish. (Continuous things —
  spinners, indeterminate progress — are the exception and use `linear`.)

**Easing — never `linear` for UI:**
- **ease-out** for things *appearing / arriving* (decelerate into place) — the most
  responsive-feeling curve; default for enter + hover-in.
- **ease-in** for things *leaving*.
- **ease-in-out** for moving/morphing between two on-screen states.
- A subtle spring/overshoot is fine for playful affordances, but keep it small —
  no big bounces on a settings panel.

**What to animate:** `opacity`, `transform` (scale/translate), and `color`/`background`
— these are cheap and smooth. **Avoid animating layout sizes** (a layout's `width`/
`height`, padding) — animate `transform-scale-x/y` with a `transform-origin` at the
anchor instead, so the whole subtree scales together without reflowing each frame.

**Patterns:**
- **Hover/press feedback is mandatory.** Every interactive element animates its
  `background` (and/or a small `scale`) on `has-hover`/`pressed`. A button with no
  transition feels broken.
- **Overlay enter/exit:** fade `opacity` 0→1 + scale ~0.96→1 from a `transform-origin`
  at the trigger (a dropdown grows from its bar entry; a dock menu from the icon).
  **Asymmetric:** ramp open with ease-in-out, snap closed faster with ease-out.
- **Keep it warm:** an overlay that's always rendered (opacity 0 when closed) avoids a
  first-frame GPU stall — animate `opacity`/`scale`, don't create/destroy.
- **Restraint:** motion draws the eye, so animate the *one* thing that changed. Don't
  stagger-animate a whole list on every keystroke; don't animate decoration.

**Slint idioms:**
```slint
// Property-level: any change to `background` tweens.
Rectangle {
    background: touch.has-hover ? #ffffff26 : #ffffff14;
    animate background { duration: 120ms; easing: ease-out; }
}
// Enter/exit via states + a scale wrapper (no layout reflow):
property <float> vis: 0.0;
states [ open when root.open: { vis: 1.0;
    in  { animate vis { duration: 240ms; easing: ease-in-out; } }   // ramp open
    out { animate vis { duration: 160ms; easing: ease-out;    } } } // snap closed
]
Rectangle {
    opacity: root.vis;
    transform-scale-x: 0.96 + 0.04 * root.vis;
    transform-scale-y: 0.96 + 0.04 * root.vis;
    transform-origin: { x: root.origin-x; y: root.origin-y; }
}
```
Put durations + curves in a **motion token global** (this repo's `Tokens.slint` has
`motion-*` — reference them, don't retype `240ms` everywhere). One named scale for
fast/medium/overlay keeps the whole UI feeling coherent.

## 7. Review checklist

Before calling a component done, check:
- [ ] Every spacing value is on the 4px scale (no `13`, `7`, `22`).
- [ ] Container padding ≥ inter-child spacing; edges aren't cramped.
- [ ] Related items closer than unrelated ones (proximity reads).
- [ ] Title/body/secondary differ by weight + opacity, not just size.
- [ ] Interactive elements ≥ 36px with a hover state **that animates** (80–150ms ease-out).
- [ ] State changes / overlays animate (opacity + transform, not layout; ease-out, <300ms).
- [ ] No `linear` easing on UI; durations on a named motion scale, not retyped magic ms.
- [ ] Radii nest (inner < outer); one soft downward shadow.
- [ ] If you removed it all and squinted: does it *breathe*?

## Worked example — a notification card

Bad (cramped): `padding: 14px`, title↔body `3px`, buttons flush under body at the
same y, button height ≈ text. Reads as a wall.

Good:
```slint
Rectangle {                       // card
    border-radius: 14px;
    VerticalLayout {
        padding: 16px;            // lg — equal, generous
        spacing: 16px;           // body-group ↔ buttons (proximity: far)
        HorizontalLayout {
            spacing: 12px;       // icon ↔ text
            Image { width: 36px; height: 36px; }
            VerticalLayout {
                spacing: 4px;    // xs — title ↔ body (proximity: near)
                Text { font-size: 15px; font-weight: 700; }
                Text { font-size: 13px; color: #ffffffa6; }   // dimmed
            }
        }
        HorizontalLayout {
            spacing: 8px;        // between buttons
            // each button: height 38px, radius 8px, hover step, centered label
        }
    }
}
```
The win is entirely spacing + rhythm + a dimmed secondary — same content, designed.
