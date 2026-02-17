---
title: Installation
---

## Homebrew (macOS/Linux)

```bash
brew install tylerbutler/tap/repoverlay
```

## Shell installer (macOS/Linux)

```bash
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/tylerbutler/repoverlay/releases/latest/download/repoverlay-installer.sh | sh
```

## PowerShell installer (Windows)

```powershell
irm https://github.com/tylerbutler/repoverlay/releases/latest/download/repoverlay-installer.ps1 | iex
```

## Cargo

```bash
# Install a pre-built binary (faster)
cargo binstall repoverlay

# Build from source
cargo install repoverlay
```

## Manual

Download the pre-built binaries for your platform at
[repoverlay releases](https://github.com/tylerbutler/repoverlay/releases).