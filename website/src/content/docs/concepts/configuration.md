---
title: Configuration
sidebar:
  order: 5
---

<!-- TODO: Document global config at ~/.config/repoverlay/config.ccl -->
<!-- TODO: Add more examples of mappings and directories -->

repoverlay uses a configuration file called `repoverlay.ccl` to control how overlays are applied. This file is optional — without it, all files in the overlay directory are symlinked with the same relative paths.

## The `repoverlay.ccl` file

Place a `repoverlay.ccl` in the root of your overlay directory:

```
overlay =
  name = my-config

/= Rename files when applying
mappings =
  .envrc.template = .envrc
  vscode-settings.json = .vscode/settings.json

/= Symlink entire directories as a unit
directories =
  = .claude
  = scratch
```

## Overlay name

The `overlay.name` field sets the name used to identify this overlay in status and remove commands. If omitted, the directory name is used.

## Mappings

The `mappings` section lets you **rename files** during apply. Each entry maps a source filename to a destination path:

```
mappings =
  .envrc.template = .envrc
```

This is useful when the overlay source uses different filenames than the target repo expects.

## Directories

The `directories` section lists directories that should be symlinked (or copied) **as a unit** rather than walking individual files:

```
directories =
  = .claude
  = .vscode
```

This is important for directories like `.claude/` where the entire directory tree should be managed atomically.

## Configuration format

repoverlay uses [CCL (Colon Configuration Language)](https://ccl.tylerbutler.com/) for its configuration files. CCL uses `=` for key-value pairs and indentation for nesting. Lines starting with `/=` are comments.
