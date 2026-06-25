---
target: website/src/pages/index.astro
total_score: 29
p0_count: 0
p1_count: 3
timestamp: 2026-06-14T23-23-46Z
slug: website-src-pages-index-astro
---
#### Design Health Score

| # | Heuristic | Score | Key Issue |
|---|-----------|-------|-----------|
| 1 | Visibility of System Status | 3 | Copy feedback exists, but is visual text-only and not announced; release/version status appears stale. |
| 2 | Match System / Real World | 4 | Strong developer-language fit with real commands, file paths, and git concepts. |
| 3 | User Control and Freedom | 3 | Static page has clear navigation, but the primary path sends users away before prerequisites are visible. |
| 4 | Consistency and Standards | 3 | Cohesive tokens and components; the gradient feature cell and repeated CTA/card grammar drift toward template behavior. |
| 5 | Error Prevention | 2 | Clipboard failure is swallowed; newcomer command examples assume setup context that may not exist yet. |
| 6 | Recognition Rather Than Recall | 3 | Commands and file examples are visible, but install/source prerequisites are not close enough to the first action. |
| 7 | Flexibility and Efficiency | 2 | Docs/CLI links help, but the homepage lacks a fast install/copy path for power users. |
| 8 | Aesthetic and Minimalist Design | 3 | Clean, focused, and readable; still leans on familiar dev-tool split hero + bento + final CTA structure. |
| 9 | Error Recovery | 2 | Very little interactive recovery surface; copy failure and JS-disabled reveal fallback are weak. |
| 10 | Help and Documentation | 4 | Docs, Quick Start, CLI reference, and footer links are easy to find. |
| **Total** | | **29/40** | **Good: solid foundation, but the top issues are trust and distinction.** |

#### Anti-Patterns Verdict

**LLM assessment**: This does not immediately scream "AI made this." The clay accent, Metropolis type, concrete `.git/info/exclude` examples, and restrained shell panel give it a real identity. The remaining slop risk is structural: split hero with terminal mock, bento feature grid, "What you get," three-step flow, and final CTA are the median developer-tool landing grammar. It is competent and tasteful, but not yet unmistakably repoverlay.

**Deterministic scan**: `detect.mjs --json website/src/pages/index.astro` returned `[]` with exit code 0. No detector findings. The scan did not catch the higher-order issues: JS-gated reveal content, conversion-path ambiguity, stale release copy, or generic landing composition.

**Visual overlays**: Not available in this session. Browser automation is unavailable, so no user-visible `[Human]` overlay was created.

#### Overall Impression

The page is credible and clear. It communicates the product better than most CLI landing pages because it uses concrete workflow artifacts instead of abstract claims. The biggest opportunity is to make the "Local Workbench" idea visible as a signature interaction or diagram rather than relying on a familiar terminal-plus-cards structure.

#### What's Working

1. **The problem framing is strong.** "Some files don't belong in the repo" names a real developer tension quickly and naturally.
2. **The page uses concrete evidence.** `.git/info/exclude`, `.claude/`, `CLAUDE.md`, and `repoverlay restore` make the product feel real.
3. **The color and type are disciplined.** The clay accent is distinctive enough without becoming noisy, and Metropolis gives the page a sturdy, practical voice.

#### Priority Issues

1. **[P1] Reveal motion can make content disappear when JavaScript fails**
   - **Why it matters**: `.reveal` starts at `opacity: 0`; if JS is disabled, delayed, blocked, or errors before adding `.in`, primary content can remain invisible. This violates the design-system rule that reveal animations must enhance an already-visible default.
   - **Fix**: Make content visible by default. Only apply hidden pre-reveal styles after an early enhancement class is present, e.g. `html.js .reveal { opacity: 0; ... }`, with the class added before render-sensitive content or with a no-JS-safe fallback.
   - **Suggested command**: `$impeccable animate website/src/pages/index.astro`

2. **[P1] The first action skips too much prerequisite context**
   - **Why it matters**: The hero CTA says "Get started," but the visible command is `repoverlay apply claude-config`, which assumes installation, source discovery, and an overlay name the user already understands. Jordan the first-timer gets a motivating page, then has to infer the real first step.
   - **Fix**: Put a minimal "install → browse → apply" path directly in the hero or immediately below it. Keep the power-user command, but make the newcomer path explicit and copyable.
   - **Suggested command**: `$impeccable clarify website/src/pages/index.astro`

3. **[P1] The brand still borrows too much generic developer-tool landing grammar**
   - **Why it matters**: Split hero, terminal mock, bento features, three-step cards, and final CTA are familiar enough that the page risks feeling polished but interchangeable. PRODUCT.md asks for "clear, practical, quietly opinionated"; the page is clear, but not yet opinionated enough.
   - **Fix**: Replace one template section with a signature repoverlay artifact: a working-tree overlay map, `.git/info/exclude` before/after, or a "repo checkout bench" diagram showing source → symlink/copy → exclude → restore.
   - **Suggested command**: `$impeccable bolder website/src/pages/index.astro`

4. **[P2] Trust cues are weaker than the product deserves**
   - **Why it matters**: The page hard-codes `release = "0.14.2"` while the crate appears to be newer, and clipboard failure is silently swallowed. A tool about safe local repo state cannot afford stale or silent UX.
   - **Fix**: Drive release copy from a single source or remove the version from the homepage banner. Add accessible copy feedback with `aria-live`, and surface clipboard failure with a small inline message.
   - **Suggested command**: `$impeccable harden website/src/pages/index.astro`

5. **[P2] Mobile and touch affordances need a pass**
   - **Why it matters**: The small copy button and dense shell/card surfaces are fine on desktop but weak for Casey on mobile. Touch targets and command overflow should be verified, not assumed.
   - **Fix**: Increase the copy target to at least 44px high on touch breakpoints, verify shell wrapping/scroll behavior, and ensure the primary CTA remains thumb-reachable after the hero stacks.
   - **Suggested command**: `$impeccable adapt website/src/pages/index.astro`

#### Persona Red Flags

**Jordan (First-Timer)**: The page explains the problem well, but the first visible command assumes Jordan already knows what `claude-config` is and how sources are registered. Jordan will click Quick Start rather than act from the homepage; that is acceptable, but the page should not imply the hero command is the first step.

**Riley (Stress Tester)**: Riley will notice the stale release banner and silent clipboard failure. Those are small individually, but they undermine confidence in a product that claims reversible, reliable state management.

**Casey (Distracted Mobile User)**: Casey gets a stacked layout and simplified nav, but the tiny copy control and horizontally scrolling shell need touch-specific hardening. A mobile user should not need precision tapping to copy the one command presented as evidence.

**Repo-hopping Developer**: This project-specific persona wants the fastest path from "new checkout" to "my config is back." The homepage currently sells that promise, but the visible flow does not show source setup, install, or restore-before/after state in one tight path.

#### Minor Observations

- The `features` object uses `variant: "gradient"` for the restore cell, but the visual system explicitly avoids ornamental gradient-card behavior. The current gradient is restrained, but the naming and treatment invite future drift.
- The nav has no active state on the homepage; acceptable for a landing page, weaker if this shell expands.
- The "View source" final CTA is useful for developers, but "Quick Start" probably deserves equal or greater end-of-page prominence.
- The page title is strong; the `description` copy could be slightly more precise by naming `.git/info/exclude` earlier.

#### Questions to Consider

- What would be unmistakably repoverlay if the terminal mock disappeared?
- Can the homepage teach the complete first successful moment in one glance: install, find overlay, apply, excluded from git?
- Should the hero artifact show a before/after working tree instead of a success log?
- If reliability is the brand promise, why is release freshness manually maintained in the page source?
