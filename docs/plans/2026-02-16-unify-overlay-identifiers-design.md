# Design: Unify Overlay Identifier Types

**Issue:** #112
**Date:** 2026-02-16
**Approach:** Bottom-up, type-first (Approach A)

## Summary

Replace raw `String` / `&str` overlay identifiers with structured types throughout the codebase. This eliminates fragile string parsing, prevents format-mismatch bugs (like #100), and adds compile-time safety for overlay identity comparisons.

## Phase 1: Unify listing functions

1. Add `Display` impl to `AvailableOverlay` — formats as `"org/repo/name"`
2. Add `AvailableOverlay::full_path()` method
3. Change `list_overlays_from_path()` to return `Vec<AvailableOverlay>`
4. Update `list_overlays_from_cached_repo()` accordingly

## Phase 2: Clean up consumers

5. Update `select_overlays_interactive()` to accept `&[AvailableOverlay]`, return `Vec<AvailableOverlay>`
6. Thread `AvailableOverlay` through GitHub two-part resolution path (~lib.rs:345)
7. Remove or reduce `parse_overlay_path()` usage
8. Update `format_overlay_path()` to accept `&AvailableOverlay`
9. Evaluate `SourceManager::list_overlays_for_repo()` — keep `Vec<String>` or upgrade

## Phase 3: Newtype wrappers

10. Introduce `OverlayName(String)` for normalized overlay names
11. Consider `OverlayPath { org, repo, name }` as a leaner identity type

## Non-goals

- `SelectableItem.id` staying `String` is fine (generic UI type)
- `list_applied_overlays()` returning `Vec<String>` is acceptable per issue scope, but Phase 3 may wrap it in `OverlayName`
