# Command-Line Help for `repoverlay`

This document contains the help content for the `repoverlay` command-line program.

**Command Overview:**

* [`repoverlay`↴](#repoverlay)
* [`repoverlay apply`↴](#repoverlay-apply)
* [`repoverlay remove`↴](#repoverlay-remove)
* [`repoverlay status`↴](#repoverlay-status)
* [`repoverlay restore`↴](#repoverlay-restore)
* [`repoverlay update`↴](#repoverlay-update)
* [`repoverlay create`↴](#repoverlay-create)
* [`repoverlay move`↴](#repoverlay-move)
* [`repoverlay switch`↴](#repoverlay-switch)
* [`repoverlay cache`↴](#repoverlay-cache)
* [`repoverlay cache list`↴](#repoverlay-cache-list)
* [`repoverlay cache remove`↴](#repoverlay-cache-remove)
* [`repoverlay cache path`↴](#repoverlay-cache-path)
* [`repoverlay browse`↴](#repoverlay-browse)
* [`repoverlay sync`↴](#repoverlay-sync)
* [`repoverlay edit`↴](#repoverlay-edit)
* [`repoverlay edit add`↴](#repoverlay-edit-add)
* [`repoverlay edit remove`↴](#repoverlay-edit-remove)
* [`repoverlay source`↴](#repoverlay-source)
* [`repoverlay source add`↴](#repoverlay-source-add)
* [`repoverlay source list`↴](#repoverlay-source-list)
* [`repoverlay source remove`↴](#repoverlay-source-remove)
* [`repoverlay marketplace`↴](#repoverlay-marketplace)
* [`repoverlay marketplace add`↴](#repoverlay-marketplace-add)
* [`repoverlay marketplace list`↴](#repoverlay-marketplace-list)
* [`repoverlay marketplace remove`↴](#repoverlay-marketplace-remove)
* [`repoverlay profile`↴](#repoverlay-profile)
* [`repoverlay profile list`↴](#repoverlay-profile-list)
* [`repoverlay profile show`↴](#repoverlay-profile-show)
* [`repoverlay profile apply`↴](#repoverlay-profile-apply)
* [`repoverlay profile status`↴](#repoverlay-profile-status)
* [`repoverlay profile remove`↴](#repoverlay-profile-remove)
* [`repoverlay library`↴](#repoverlay-library)
* [`repoverlay library list`↴](#repoverlay-library-list)
* [`repoverlay library import`↴](#repoverlay-library-import)
* [`repoverlay library export`↴](#repoverlay-library-export)
* [`repoverlay library remove`↴](#repoverlay-library-remove)
* [`repoverlay copilot`↴](#repoverlay-copilot)
* [`repoverlay claude`↴](#repoverlay-claude)
* [`repoverlay completions`↴](#repoverlay-completions)

## `repoverlay`

Overlay config files into git repositories without committing them

**Usage:** `repoverlay [COMMAND]`

### Subcommands

* `apply` — Apply an overlay to a git repository (scripting / power-user)
* `remove` — Remove applied overlay(s)
* `status` — Show the status of applied overlays
* `restore` — Restore overlays after git clean or other removal
* `update` — Update applied overlays from remote sources
* `create` — Create a new overlay from files in a repository
* `move` — Move an overlay to a new location
* `switch` — Switch to a different overlay (removes all existing overlays first)
* `cache` — Manage the overlay cache
* `browse` — Browse and apply overlays interactively (recommended)
* `sync` — Sync changes from an applied overlay back to the overlay repo
* `edit` — Edit an existing applied overlay
* `source` — Manage overlay sources (for multi-source configurations)
* `marketplace` — Manage plugin marketplaces
* `profile` — Manage repository profiles
* `library` — Manage the in-repo overlay library
* `copilot` — Run GitHub Copilot with one or more profiles applied for the process lifetime
* `claude` — Run Claude with one or more profiles applied for the process lifetime
* `completions` — Generate shell completions



## `repoverlay apply`

Apply an overlay to a git repository (scripting / power-user)

Applies an overlay directly from a path, GitHub URL, or configured source. Intended for scripting and automation workflows.

For interactive discovery and application, use `repoverlay browse` instead — it lists available overlays and lets you select which to apply.

**Usage:** `repoverlay apply [OPTIONS] <SOURCE>`

### Arguments

* `<SOURCE>` — Overlay source: local path, GitHub URL, or configured source reference

   Supported formats: ./my-overlay             (local path) /absolute/path           (local absolute path) <https://github.com/owner/repo> <https://github.com/owner/repo/tree/main/overlays/rust> org/repo/overlay-name    (configured source reference)

### Options

* `-t`, `--target <TARGET>` — Target repository directory (defaults to current directory)
* `--copy` — Force copy mode instead of symlinks (default on Windows)
* `-n`, `--name <NAME>` — Override the overlay name (defaults to config name or directory name)
* `-r`, `--ref <REF>` — Git ref (branch, tag, or commit) to use (GitHub sources only)
* `--no-update` — Skip updating cached/overlay repositories before applying
* `--force` [alias: `overwrite`] — Overwrite existing files and re-apply same-name overlays
* `--skip-conflicts` — Skip conflicting files silently, continue with non-conflicting files
* `-i`, `--interactive` — Prompt interactively for each conflict (overwrite, skip, diff, or abort)
* `--merge` — Deep merge conflicting JSON files instead of failing
* `--from <SOURCE>` — Use a specific overlay source instead of priority order (multi-source configs only)
* `--dry-run` — Show what would be applied without making changes



## `repoverlay remove`

Remove applied overlay(s)

**Usage:** `repoverlay remove [OPTIONS] [NAME]`

### Arguments

* `<NAME>` — Name of the overlay to remove

### Options

* `-t`, `--target <TARGET>` — Target repository directory (defaults to current directory)
* `--all` — Remove all applied overlays
* `--dry-run` — Show what would be removed without making changes
* `-i`, `--interactive` — Interactive selection mode



## `repoverlay status`

Show the status of applied overlays

**Usage:** `repoverlay status [OPTIONS]`

### Options

* `-t`, `--target <TARGET>` — Target repository directory (defaults to current directory)
* `-n`, `--name <NAME>` — Show only a specific overlay
* `--json` — Output as JSON for scripting and CI integration
* `-q`, `--quiet` — Quiet mode: exit code only (0 = overlays applied, 1 = none)



## `repoverlay restore`

Restore overlays after git clean or other removal

**Usage:** `repoverlay restore [OPTIONS]`

### Options

* `-t`, `--target <TARGET>` — Target repository directory (defaults to current directory)
* `--dry-run` — Show what would be restored without applying
* `--force` [alias: `overwrite`] — Overwrite existing files during restore
* `--skip-conflicts` — Skip conflicting files silently during restore
* `-i`, `--interactive` — Prompt interactively for each conflict during restore
* `--merge` — Deep merge conflicting JSON files instead of failing



## `repoverlay update`

Update applied overlays from remote sources

**Usage:** `repoverlay update [OPTIONS] [NAME]`

### Arguments

* `<NAME>` — Name of the overlay to update (updates all GitHub overlays if not specified)

### Options

* `-t`, `--target <TARGET>` — Target repository directory (defaults to current directory)
* `--dry-run` — Check for updates without applying them
* `--force` [alias: `overwrite`] — Overwrite existing files during update
* `--skip-conflicts` — Skip conflicting files silently during update
* `-i`, `--interactive` — Prompt interactively for each conflict during update
* `--merge` — Deep merge conflicting JSON files instead of failing



## `repoverlay create`

Create a new overlay from files in a repository

Examples: repoverlay create my-overlay              # Detects org/repo from git remote repoverlay create org/repo/my-overlay     # Explicit target repoverlay create --output ./output        # Write to local directory

**Usage:** `repoverlay create [OPTIONS] [NAME]`

### Arguments

* `<NAME>` — Overlay name or full path (org/repo/name)

   Short form: `my-overlay` - detects org/repo from git remote Full form: `org/repo/name` - uses explicit target Omit when using --output for local directory output

### Options

* `-i`, `--include <INCLUDE>` — Include files/directories or glob patterns (can be specified multiple times)
* `-s`, `--source <SOURCE>` — Source repository to extract files from (defaults to current directory)
* `-t`, `--target <TARGET>` — Target repository directory (defaults to current directory)
* `-o`, `--output <OUTPUT>` — Output directory for local overlay creation (no overlay repo required)
* `--into <DEST>` — Create the overlay directly into a destination ("library")
* `--no-apply` — Skip applying the overlay after creating into the library
* `--global` — Create a global overlay (applies to any repository)

   Scaffolds into the `@global/<name>/` namespace of the overlay repo instead of `org/repo/<name>/`, skipping git-remote target detection.
* `--dry-run` — Show what would be created without creating files
* `-y`, `--yes` — Skip interactive prompts, use defaults
* `-f`, `--force` — Force overwrite if overlay already exists



## `repoverlay move`

Move an overlay to a new location

Relocates an overlay's source files and updates applied state references. Destinations: "library", "source:<name>", or a filesystem path.

Examples: repoverlay move my-overlay --to library repoverlay move my-overlay --to /path/to/dir repoverlay move my-overlay --to source:shared-repo

**Usage:** `repoverlay move [OPTIONS] --to <TO> <OVERLAY>`

### Arguments

* `<OVERLAY>` — Name of the applied overlay to move

### Options

* `--to <TO>` — Destination: "library", "source:<name>", or a filesystem path
* `-t`, `--target <TARGET>` — Target repository directory (defaults to current directory)
* `--force` — Overwrite if destination already exists
* `--name <NAME>` — Rename the overlay at the destination
* `--target-repo <TARGET_REPO>` — Override the target repository (org/repo format, e.g. acme/my-app)

   Used when moving to a named source and the git origin remote cannot be parsed.
* `--dry-run` — Show what would happen without making changes



## `repoverlay switch`

Switch to a different overlay (removes all existing overlays first)

**Usage:** `repoverlay switch [OPTIONS] <SOURCE>`

### Arguments

* `<SOURCE>` — Path to overlay source directory OR GitHub URL

### Options

* `-t`, `--target <TARGET>` — Target repository directory (defaults to current directory)
* `--copy` — Force copy mode instead of symlinks (default on Windows)
* `-n`, `--name <NAME>` — Override the overlay name
* `-r`, `--ref <REF>` — Git ref (branch, tag, or commit) to use (GitHub sources only)
* `--no-update` — Skip updating cached/overlay repositories before switching
* `--force` [alias: `overwrite`] — Overwrite existing repo files when applying the new overlay
* `--skip-conflicts` — Skip conflicting repo files silently when applying the new overlay
* `-i`, `--interactive` — Prompt interactively for each conflict when applying the new overlay
* `--merge` — Deep merge conflicting JSON files instead of failing
* `--dry-run` — Show what would be switched without making changes



## `repoverlay cache`

Manage the overlay cache

**Usage:** `repoverlay cache <COMMAND>`

### Subcommands

* `list` — List cached GitHub repositories and configured-source clones
* `remove` — Remove cached GitHub repositories and configured-source clones
* `path` — Show cache location



## `repoverlay cache list`

List cached GitHub repositories and configured-source clones

**Usage:** `repoverlay cache list`



## `repoverlay cache remove`

Remove cached GitHub repositories and configured-source clones

**Usage:** `repoverlay cache remove [OPTIONS] [REPO]`

### Arguments

* `<REPO>` — Cached GitHub repository to remove (format: owner/repo)

### Options

* `--source <SOURCE>` — Cached source clone to remove by source name
* `-a`, `--all` — Remove all cached GitHub repositories and configured-source clones
* `-y`, `--yes` — Skip confirmation prompt when removing all cache entries



## `repoverlay cache path`

Show cache location

**Usage:** `repoverlay cache path`



## `repoverlay browse`

Browse and apply overlays interactively (recommended)

Lists available overlays from configured sources and lets you select which to apply. This is the easiest way to discover and apply overlays. To add sources, run `repoverlay source add <path-or-url>`.

**Usage:** `repoverlay browse [OPTIONS] [SOURCE]`

### Arguments

* `<SOURCE>` — Overlay source (GitHub username, owner/repo, or URL)

   Browse overlays from this source without adding it as a configured source. If omitted, uses configured sources.

### Options

* `-f`, `--filter <FILTER>` — Filter by target repository (format: org/repo)
* `--no-update` — Skip updating overlay repo before listing
* `-t`, `--target <TARGET>` — Target repository directory (defaults to current directory)
* `--no-interactive` — Disable interactive selection (just list overlays)
* `--dry-run` — Show what would be applied without making changes
* `--show-all` — Show all overlays, including those for other repositories



## `repoverlay sync`

Sync changes from an applied overlay back to the overlay repo

Examples: repoverlay sync my-overlay          # Detects org/repo from git remote repoverlay sync org/repo/my-overlay # Explicit target repoverlay sync --all               # Sync all applied overlays

**Usage:** `repoverlay sync [OPTIONS] [NAME]`

### Arguments

* `<NAME>` — Overlay name or full path (org/repo/name)

   Short form: `my-overlay` - detects org/repo from git remote Full form: `org/repo/name` - uses explicit values

### Options

* `-t`, `--target <TARGET>` — Target repository directory (defaults to current directory)
* `--all` — Sync all applied overlays from the overlay repo
* `--dry-run` — Show what would be synced without making changes



## `repoverlay edit`

Edit an existing applied overlay

With no subcommand, launches interactive file selection. If no overlay name is given, prompts to select from applied overlays.

Examples: repoverlay edit                                     # Pick overlay, then edit repoverlay edit my-overlay                          # Interactive file selection repoverlay edit add my-overlay newfile.txt          # Add files repoverlay edit add my-overlay file1.txt file2.txt  # Add multiple files repoverlay edit remove my-overlay oldfile.txt       # Remove files

**Usage:** `repoverlay edit [OPTIONS] [NAME]
       edit <COMMAND>`

### Subcommands

* `add` — Add files to an applied overlay
* `remove` — Remove files from an applied overlay

### Arguments

* `<NAME>` — Overlay name or full path (org/repo/name)

   Short form: `my-overlay` - detects org/repo from git remote Full form: `org/repo/name` - uses explicit values

### Options

* `-t`, `--target <TARGET>` — Target repository directory (defaults to current directory)
* `--dry-run` — Show what would change without making changes



## `repoverlay edit add`

Add files to an applied overlay

Examples: repoverlay edit add my-overlay newfile.txt repoverlay edit add my-overlay file1.txt file2.txt repoverlay edit add org/repo/my-overlay newfile.txt

**Usage:** `repoverlay edit add [OPTIONS] <NAME> [FILES]...`

### Arguments

* `<NAME>` — Overlay name or full path (org/repo/name)

   Short form: `my-overlay` - detects org/repo from git remote Full form: `org/repo/name` - uses explicit values
* `<FILES>` — Files to add to the overlay

### Options

* `-t`, `--target <TARGET>` — Target repository directory (defaults to current directory)
* `--dry-run` — Show what would change without making changes



## `repoverlay edit remove`

Remove files from an applied overlay

Examples: repoverlay edit remove my-overlay oldfile.txt repoverlay edit remove my-overlay file1.txt file2.txt repoverlay edit remove org/repo/my-overlay oldfile.txt

**Usage:** `repoverlay edit remove [OPTIONS] <NAME> [FILES]...`

### Arguments

* `<NAME>` — Overlay name or full path (org/repo/name)

   Short form: `my-overlay` - detects org/repo from git remote Full form: `org/repo/name` - uses explicit values
* `<FILES>` — Files to remove from the overlay

### Options

* `-t`, `--target <TARGET>` — Target repository directory (defaults to current directory)
* `--dry-run` — Show what would change without making changes



## `repoverlay source`

Manage overlay sources (for multi-source configurations)

**Usage:** `repoverlay source <COMMAND>`

### Subcommands

* `add` — Add a new overlay source
* `list` — List configured overlay sources
* `remove` — Remove an overlay source



## `repoverlay source add`

Add a new overlay source

**Usage:** `repoverlay source add [OPTIONS] <SOURCE>`

### Arguments

* `<SOURCE>` — Source: Git URL, GitHub shorthand (owner/repo), GitHub username, or local path (./path)

### Options

* `--name <NAME>` — Name for this source (defaults to repo/directory name)



## `repoverlay source list`

List configured overlay sources

**Usage:** `repoverlay source list`



## `repoverlay source remove`

Remove an overlay source

**Usage:** `repoverlay source remove <NAME>`

### Arguments

* `<NAME>` — Name of the source to remove



## `repoverlay marketplace`

Manage plugin marketplaces

**Usage:** `repoverlay marketplace <COMMAND>`

### Subcommands

* `add` — Register a plugin marketplace
* `list` — List registered marketplaces
* `remove` — Remove a registered marketplace



## `repoverlay marketplace add`

Register a plugin marketplace

**Usage:** `repoverlay marketplace add [OPTIONS] <NAME> <URL>`

### Arguments

* `<NAME>` — Name for this marketplace (used in `marketplace/plugin` references)
* `<URL>` — Marketplace git URL or GitHub shorthand (owner/repo)

### Options

* `--yes` — Skip the confirmation prompt



## `repoverlay marketplace list`

List registered marketplaces

**Usage:** `repoverlay marketplace list`



## `repoverlay marketplace remove`

Remove a registered marketplace

**Usage:** `repoverlay marketplace remove <NAME>`

### Arguments

* `<NAME>` — Name of the marketplace to remove



## `repoverlay profile`

Manage repository profiles

**Usage:** `repoverlay profile <COMMAND>`

### Subcommands

* `list` — List configured profiles
* `show` — Show a configured profile
* `apply` — Apply a profile persistently
* `status` — Show applied profile state
* `remove` — Remove an applied profile



## `repoverlay profile list`

List configured profiles

**Usage:** `repoverlay profile list [OPTIONS]`

### Options

* `-t`, `--target <TARGET>` — Target repository directory (defaults to current directory)



## `repoverlay profile show`

Show a configured profile

**Usage:** `repoverlay profile show [OPTIONS] <NAME>`

### Arguments

* `<NAME>` — Profile name

### Options

* `-t`, `--target <TARGET>` — Target repository directory (defaults to current directory)



## `repoverlay profile apply`

Apply a profile persistently

**Usage:** `repoverlay profile apply [OPTIONS] --harness <HARNESS> <NAME>`

### Arguments

* `<NAME>` — Profile name

### Options

* `--harness <HARNESS>`

  Possible values: `copilot`, `claude`

* `-t`, `--target <TARGET>` — Target repository directory (defaults to current directory)



## `repoverlay profile status`

Show applied profile state

**Usage:** `repoverlay profile status [OPTIONS]`

### Options

* `-t`, `--target <TARGET>` — Target repository directory (defaults to current directory)
* `--harness <HARNESS>`

  Possible values: `copilot`, `claude`




## `repoverlay profile remove`

Remove an applied profile

**Usage:** `repoverlay profile remove [OPTIONS] --harness <HARNESS> <NAME>`

### Arguments

* `<NAME>` — Profile name

### Options

* `--harness <HARNESS>`

  Possible values: `copilot`, `claude`

* `-t`, `--target <TARGET>` — Target repository directory (defaults to current directory)



## `repoverlay library`

Manage the in-repo overlay library

**Usage:** `repoverlay library <COMMAND>`

### Subcommands

* `list` — List overlays in the library
* `import` — Import an overlay into the library
* `export` — Export an overlay from the library
* `remove` — Remove an overlay from the library



## `repoverlay library list`

List overlays in the library

**Usage:** `repoverlay library list [OPTIONS]`

### Options

* `-t`, `--target <TARGET>` — Target repository directory (defaults to current directory)



## `repoverlay library import`

Import an overlay into the library

**Usage:** `repoverlay library import [OPTIONS] <SOURCE>`

### Arguments

* `<SOURCE>` — Overlay source (path, GitHub URL, or org/repo/name)

### Options

* `--name <NAME>` — Name for the imported overlay (defaults to source name)
* `-f`, `--force` — Force overwrite if overlay already exists
* `-t`, `--target <TARGET>` — Target repository directory (defaults to current directory)



## `repoverlay library export`

Export an overlay from the library

**Usage:** `repoverlay library export [OPTIONS] --to <DEST> <OVERLAY>`

### Arguments

* `<OVERLAY>` — Name of the overlay to export

### Options

* `--to <DEST>` — Destination path
* `-t`, `--target <TARGET>` — Target repository directory (defaults to current directory)



## `repoverlay library remove`

Remove an overlay from the library

**Usage:** `repoverlay library remove [OPTIONS] <OVERLAY>`

### Arguments

* `<OVERLAY>` — Name of the overlay to remove

### Options

* `-f`, `--force` — Force removal even if overlay is currently applied
* `-t`, `--target <TARGET>` — Target repository directory (defaults to current directory)



## `repoverlay copilot`

Run GitHub Copilot with one or more profiles applied for the process lifetime

**Usage:** `repoverlay copilot [OPTIONS] --profile <PROFILES> [-- <EXTRA_ARGS>...]`

### Arguments

* `<EXTRA_ARGS>` — Extra arguments forwarded to the Copilot harness

### Options

* `--profile <PROFILES>` — Profile name to apply while Copilot runs (repeat to apply several)
* `-t`, `--target <TARGET>` — Target repository directory (defaults to current directory)



## `repoverlay claude`

Run Claude with one or more profiles applied for the process lifetime

**Usage:** `repoverlay claude [OPTIONS] --profile <PROFILES> [-- <EXTRA_ARGS>...]`

### Arguments

* `<EXTRA_ARGS>` — Extra arguments forwarded to the Claude harness

### Options

* `--profile <PROFILES>` — Profile name to apply while Claude runs (repeat to apply several)
* `-t`, `--target <TARGET>` — Target repository directory (defaults to current directory)



## `repoverlay completions`

Generate shell completions

**Usage:** `repoverlay completions <SHELL>`

### Arguments

* `<SHELL>` — Shell to generate completions for

  Possible values: `bash`, `elvish`, `fish`, `powershell`, `zsh`




<hr/>

<small><i>
    This document was generated automatically by
    <a href="https://crates.io/crates/clap-markdown"><code>clap-markdown</code></a>.
</i></small>
