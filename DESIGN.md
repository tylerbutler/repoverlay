---
name: repoverlay
description: Overlay config files into git repositories without committing them.
colors:
  bg-dark: "#14151f"
  bg-dark-raised: "#181a26"
  surface-dark: "#1d1f2c"
  surface-dark-translucent: "#23253480"
  text-dark: "#ebedf9"
  text-dark-secondary: "#b7bace"
  text-muted: "#8789a4"
  border-dark: "#ebedf91a"
  border-dark-strong: "#ebedf92e"
  accent-dark: "#cf6242"
  accent-deep: "#aa462c"
  accent-soft-dark: "#cf624224"
  accent-line-dark: "#cf624266"
  on-accent-dark: "#1a0f0b"
  success: "#5fb98b"
  bg-light: "#f6f7fc"
  bg-light-raised: "#eef0f8"
  surface-light: "#ffffff"
  text-light: "#161721"
  text-light-secondary: "#3a3c52"
  accent-light: "#ac482e"
  accent-light-deep: "#8f371f"
typography:
  display:
    fontFamily: "Schibsted Grotesk Variable, system-ui, sans-serif"
    fontSize: "clamp(3rem, 7.8vw, 5.4rem)"
    fontWeight: 800
    lineHeight: 1.05
    letterSpacing: "-0.04em"
  headline:
    fontFamily: "Schibsted Grotesk Variable, system-ui, sans-serif"
    fontSize: "clamp(1.9rem, 3.6vw, 2.8rem)"
    fontWeight: 800
    lineHeight: 1.05
    letterSpacing: "-0.02em"
  title:
    fontFamily: "Schibsted Grotesk Variable, system-ui, sans-serif"
    fontSize: "1.25rem"
    fontWeight: 700
    lineHeight: 1.05
    letterSpacing: "-0.02em"
  body:
    fontFamily: "Schibsted Grotesk Variable, system-ui, sans-serif"
    fontSize: "17px"
    fontWeight: 400
    lineHeight: 1.6
  label:
    fontFamily: "Schibsted Grotesk Variable, system-ui, sans-serif"
    fontSize: "0.72rem"
    fontWeight: 700
    letterSpacing: "0.13em"
  mono:
    fontFamily: "Commit Mono, ui-monospace, SFMono-Regular, Menlo, Consolas, monospace"
    fontSize: "0.86rem"
    fontWeight: 400
    lineHeight: 1.85
rounded:
  sm: "7px"
  md: "10px"
  lg: "16px"
  pill: "999px"
spacing:
  xs: "0.5rem"
  sm: "0.85rem"
  md: "1.2rem"
  lg: "1.6rem"
  xl: "2.5rem"
  section: "clamp(4.5rem, 9vw, 8rem)"
components:
  button-primary:
    backgroundColor: "{colors.accent-dark}"
    textColor: "{colors.on-accent-dark}"
    typography: "{typography.body}"
    rounded: "{rounded.pill}"
    padding: "0.8rem 1.4rem"
  button-ghost:
    backgroundColor: "{colors.surface-dark-translucent}"
    textColor: "{colors.text-dark}"
    typography: "{typography.body}"
    rounded: "{rounded.pill}"
    padding: "0.8rem 1.4rem"
  copy-button:
    backgroundColor: "{colors.surface-dark-translucent}"
    textColor: "{colors.text-dark-secondary}"
    typography: "{typography.label}"
    rounded: "{rounded.sm}"
    padding: "0.25rem 0.6rem"
  shell-panel:
    backgroundColor: "{colors.surface-dark}"
    textColor: "{colors.text-dark}"
    typography: "{typography.mono}"
    rounded: "{rounded.lg}"
    padding: "0"
  overlay-stack:
    backgroundColor: "{colors.accent-dark}"
    textColor: "{colors.text-dark}"
    rounded: "{rounded.lg}"
    padding: "0.7rem 0.82rem"
  feature-card:
    backgroundColor: "{colors.surface-dark}"
    textColor: "{colors.text-dark}"
    typography: "{typography.body}"
    rounded: "{rounded.lg}"
    padding: "1.6rem 1.6rem 1.7rem"
  code-chip:
    backgroundColor: "{colors.bg-dark-raised}"
    textColor: "{colors.text-dark-secondary}"
    typography: "{typography.mono}"
    rounded: "{rounded.sm}"
    padding: "0.3rem 0.6rem"
---

# Design System: repoverlay

## 1. Overview

**Creative North Star: "The Local Workbench"**

repoverlay's visual system should feel like the bench where a developer keeps the tools they
trust: close at hand, clearly labeled, and built for repeatable work. The current site uses a
dark technical surface, a clay-orange accent, and compact command examples to make invisible git
mechanics feel tangible rather than abstract.

The system is brand-first because the primary frontend surface is a docs and landing site, but it
should not perform like a generic marketing page. It rejects floating SaaS dashboards, hero
metrics, ornamental gradient-card grids, and vague productivity copy. The design earns trust by
showing concrete files, commands, exclusions, and restore flows.

**Key Characteristics:**

- Dark-by-default developer environment with a complete light-mode counterpart.
- Schibsted Grotesk-driven type system: one grotesk carries body and display through weight
  contrast, sturdy and readable rather than trendy.
- Clay-orange accent used as a working signal for action, success-path emphasis, and command
  prompts.
- Tonal layering, borders, and a single structural shell shadow instead of decorative depth.
- Real workflow objects as imagery: command panels, config file names, overlay flow steps, and
  stacked source/worktree/exclude artifacts.

## 2. Colors

The palette is a quiet technical dark theme warmed by a clay-orange accent, with light mode kept
cool and neutral rather than cream-toned.

### Primary

- **Workbench Clay**: The primary action and emphasis color. Use it for primary buttons, command
  prompts, release dots, active links, and sparse highlights that point to the mechanics of the
  product.
- **Deep Clay**: The lower, steadier accent. Use it for hover mixes, Starlight docs accenting, and
  places where the primary clay would feel too bright.
- **Clay Wash**: The soft accent field. Use it only as a quiet radial wash or chip background when
  the content itself deserves emphasis.

### Neutral

- **Bench Black**: The default page background. It carries the dark interface and should not be
  replaced by pure black.
- **Raised Slate**: The release bar, footer, and small code-chip ground. Use it to separate utility
  bands from the main canvas.
- **Tool Surface**: The card and shell-panel surface. It should feel close to the background, not
  like a floating glass card.
- **Ink White**: Primary dark-mode text. Use it for headings, body text, command output, and any
  text that carries meaning.
- **Quiet Lavender Gray**: Secondary text. Use it for ledes, descriptions, nav links, and footer
  copy after verifying contrast.
- **Muted Tool Gray**: Labels and dim command output. Use sparingly; never use it for body copy.
- **Cool Paper**: Light-mode background. It is intentionally cool and clean, not beige, parchment,
  or SaaS cream.

### Named Rules

**The One-Tool Accent Rule.** Clay is the only brand accent. Do not introduce blue, purple, or
rainbow developer-tool gradients unless the product itself gains a new semantic state that
requires them.

**The No Cream Rule.** Light mode stays cool and neutral. Warmth comes from the clay accent, not
from sand, bone, parchment, or ivory backgrounds.

## 3. Typography

**Display Font:** Schibsted Grotesk (variable) with system sans-serif fallback.  
**Body Font:** Schibsted Grotesk (variable, regular weight) with system sans-serif fallback.  
**Label Font:** Schibsted Grotesk (UI weights).  
**Mono Font:** Commit Mono for commands and code only, with a UI monospace fallback stack.

**Character:** Schibsted Grotesk carries the whole UI as a single variable grotesk: a tight,
engineered voice that reads as precise rather than decorative. Hierarchy comes from committed
weight contrast (regular body against 700/800 headings) plus the fluid size scale, not from a
second, near-identical sans. Commit Mono is a neutral, exact monospace used functionally for
commands and file paths, not as a costume for "developer brand." The system pairs a single
grotesk against monospace-for-code only; the two families (grotesk, mono) are shared identically
by the homepage and the docs.

### Hierarchy

- **Display** (800, `clamp(3rem, 7.8vw, 5.4rem)`, 1.05): Hero headlines and the strongest landing
  statements. Keep letter spacing at or above `-0.04em`.
- **Headline** (800, `clamp(1.9rem, 3.6vw, 2.8rem)`, 1.05): Section-level arguments such as the
  problem statement and final CTA.
- **Title** (700, `1.18rem` to `1.25rem`, 1.05): Feature cards and flow steps.
- **Body** (400, `17px`, 1.6): Default prose. Keep line length tight; ledes currently top out
  around 34ch and explanatory blocks around 42-46ch.
- **Label** (700, `0.72rem`, `0.13em`): Short utility labels only, such as config categories and
  footer group titles.
- **Command** (400, `0.86rem`, 1.85): Shell output, file paths, and command chips.

### Named Rules

**The Mono Earns Its Place Rule.** Monospace appears only when the content is literally code,
commands, file paths, or terminal output. Do not use monospace as a generic technical decoration.

## 4. Elevation

repoverlay is flat by default. Depth comes from tonal steps, fine borders, and content grouping.
The only strong shadow in the landing page is the shell panel's structural shadow, which makes
the command example feel like the primary object on the bench rather than another card.

### Shadow Vocabulary

- **Shell Object Shadow** (`0 24px 60px rgba(8, 9, 16, 0.45)`): Reserved for the hero shell or an
  equally important command/workflow artifact.
- **Accent Glow** (`radial-gradient(...)` with blur): Use behind a focal command or feature only;
  never behind every card.

### Named Rules

**The Flat-Until-Physical Rule.** A surface gets a shadow only when it is meant to read as a
physical object. Ordinary feature cells stay flat and use borders plus tonal contrast.

## 5. Components

### Buttons

- **Shape:** Full-pill actions (`999px`) with compact vertical rhythm.
- **Primary:** Workbench Clay background with on-accent text, `0.8rem 1.4rem` padding, 600 weight,
  and an inline arrow when the action advances the user.
- **Hover / Focus:** Primary buttons lift by `translateY(-2px)` and brighten slightly. Focus uses
  a visible 2px clay outline with a 3px offset.
- **Ghost:** Transparent at rest with a strong border; hover adds a clay wash and border tint
  rather than a drop shadow.

### Chips

- **Style:** Small monospace pills with 7px corners, tonal background, 1px border, and secondary
  text.
- **State:** Accent chips use Clay Wash and Clay Line only when the command or file path is the
  point of the card.

### Cards / Containers

- **Corner Style:** 16px radius for feature cells, shell panels, config grids, and major grouped
  containers.
- **Background:** Tool Surface on Bench Black, with 1px borders to define structure.
- **Shadow Strategy:** Flat for normal cards; Shell Object Shadow for the hero terminal only.
- **Border:** Use subtle dark-mode borders for separation. Avoid colored side stripes.
- **Internal Padding:** Feature cells use `1.6rem 1.6rem 1.7rem`; compact list cells use about
  `1.2rem 1.25rem`.

### Navigation

- **Style:** Sticky top nav on a blurred, saturated canvas with a 1px bottom border.
- **Typography:** Compact grotesk links with secondary text color at rest.
- **States:** Hover shifts links to primary text or Clay for icon links. The nav should stay calm;
  do not add badge clutter or dropdown theatrics.
- **Mobile:** Keep the brand and all primary docs links (Docs, Quick Start, CLI) reachable; below
  600px tighten link spacing and type rather than dropping the CLI link, and hide the brand
  wordmark only below 360px.

### Shell Panel

The shell panel is the signature brand object. It should show real commands, real output, and
file paths using the monospace stack, with Clay marking prompts and success-path emphasis. Copy
buttons are small, bordered, and utility-like; success changes text to Clay, while clipboard
failure must be visible and announced so users know to copy the commands manually.

### Overlay Stack

The overlay stack is the bolder hero-only artifact: small physical labels for overlay source,
working tree, restore state, and `.git/info/exclude` sit behind the shell panel on a Clay backing
plate. Use it when the page needs the mechanics to feel tangible; do not repeat it as generic card
decoration elsewhere.

### Motion

Motion is restrained and functional: hover lifts on actionable elements, enhancement-only
reveals for landing sections, and a blinking caret inside the shell. Reveal motion must never
gate readability; content is visible by default and JavaScript adds animation only when motion is
allowed and supported. Reduced motion removes the caret blink, reveal animation, and hover lifts.

### Iconography

- **Set:** [Tabler](https://tabler.io/icons) (outline, 2px stroke) is the single icon system across
  homepage and docs. It replaces Starlight's stock icons so the chrome reads as one cohesive,
  non-default set.
- **Mechanism:** Starlight has no supported way to swap its built-in icon registry, so the docs CSS
  (`website/src/styles/custom.css`) masks every icon-bearing `<svg>` — Starlight UI icons, callout
  icons, heading anchors, the announcement banner, and Expressive Code's frame glyphs — with a Tabler
  shape held in a `--tbl-*` token. Icons inherit `currentColor`, so they adapt to theme automatically.
- **Mapping highlights:** chevrons for carets (right-base inside `<details>` so they read ▶/▼),
  sun/moon for the appearance toggle, info-circle / bulb / alert-triangle / flame for note / tip /
  caution / danger callouts, a hash for heading anchors, and a rocket for the release banner.

### Production Hardening

The homepage release banner reads the version from root `Cargo.toml` at build time, not from a
hand-maintained string. Code and file-path treatments must allow emergency wrapping or horizontal
scrolling so long owner/repo names, translated labels, and generated paths do not create page-level
overflow. Forced-colors mode removes decorative hero layers and keeps bordered functional objects
visible.

## 6. Do's and Don'ts

### Do:

- **Do** show real workflow objects: `.git/info/exclude`, `repoverlay apply`, `repoverlay restore`,
  overlay state, and file paths.
- **Do** keep Clay rare enough to remain meaningful; use it for actions, prompts, release markers,
  and the most important command artifact.
- **Do** use the existing 16px container radius and 7-10px utility radii before inventing new
  shape values.
- **Do** keep text contrast strong in both color schemes, especially secondary prose on tinted
  dark surfaces.
- **Do** make reduced-motion behavior visible and complete; content must be readable before any
  animation enhancement runs.
- **Do** surface clipboard failures and keep release/version text sourced from build-time project
  metadata.

### Don't:

- **Don't** make repoverlay look like a generic SaaS landing page with gradient cards, hero
  metrics, floating dashboard mockups, or vague productivity promises.
- **Don't** use terminal-only hacker cosplay. Commands are evidence, not a whole aesthetic.
- **Don't** add overly playful mascots or decorative AI-assistant tropes.
- **Don't** use gradient text, colored side-stripe borders, nested glass cards, or repeated
  uppercase eyebrows above every section.
- **Don't** replace the cool light-mode palette with cream, sand, parchment, bone, ivory, or other
  warm-neutral defaults.
