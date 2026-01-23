---
name: release
description: Guide through release preparation
disable-model-invocation: true
---

Pre-release checklist for repoverlay:

1. Run full CI checks: `just ci`
2. Run security audit: `just audit`
3. Check CHANGELOG.md is updated
4. Verify version in Cargo.toml
5. Build release binary: `just release`
6. Test the release binary manually

Do NOT push or create tags - let release-plz handle that via GitHub Actions.