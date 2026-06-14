# Product

## Register

brand

## Users

repoverlay is for developers who work across multiple git repositories and carry local
configuration with them: AI assistant instructions, editor settings, environment files, and
shared dev-tooling preferences. They are usually working inside an existing project, often a
fork or a short-lived checkout, and need their preferred files present without polluting the
repository's committed history.

## Product Purpose

repoverlay overlays config files into git repositories without committing them. It lets users
define portable overlay sources, apply them by name or path, keep the applied files excluded
through `.git/info/exclude`, and restore them after cleanup. Success means a developer can move
between repos with the right local setup in place, understand exactly what changed, and undo or
recover those changes confidently.

## Brand Personality

Clear, practical, and quietly opinionated. The brand should feel like a reliable command-line
tool made by someone who understands the friction of real development workflows: precise,
unhyped, and willing to make a strong call when that makes the workflow safer.

## Anti-references

Do not make repoverlay look like a generic SaaS landing page with gradient cards, hero metrics,
floating dashboard mockups, or vague productivity promises. Avoid terminal-only hacker cosplay,
overly playful mascots, and decorative AI-assistant tropes that distract from the concrete file
and git mechanics.

## Design Principles

- Make the invisible mechanics visible: show the actual files, commands, exclusions, and state
  transitions that make the workflow trustworthy.
- Prefer developer confidence over marketing spectacle: concrete examples should carry more
  weight than abstract claims.
- Keep portability central: multi-repo use, shared sources, forks, restore, and update flows
  should feel like one coherent system rather than separate features.
- Respect local ownership: the interface should reinforce that personal config stays local,
  reversible, and outside committed history.
- Be distinct without shouting: use committed typography, color, and layout choices, but avoid
  patterns that collapse into the average developer-tool landing page.

## Accessibility & Inclusion

Target WCAG 2.2 AA. Preserve keyboard-first navigation, visible focus states, strong text
contrast, reduced-motion alternatives, and copy that explains workflows without relying only on
color, icons, or animation.
