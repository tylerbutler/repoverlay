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
* [`repoverlay create-local`↴](#repoverlay-create-local)
* [`repoverlay switch`↴](#repoverlay-switch)
* [`repoverlay cache`↴](#repoverlay-cache)
* [`repoverlay cache list`↴](#repoverlay-cache-list)
* [`repoverlay cache clear`↴](#repoverlay-cache-clear)
* [`repoverlay cache remove`↴](#repoverlay-cache-remove)
* [`repoverlay cache path`↴](#repoverlay-cache-path)
* [`repoverlay list`↴](#repoverlay-list)
* [`repoverlay sync`↴](#repoverlay-sync)
* [`repoverlay edit`↴](#repoverlay-edit)
* [`repoverlay source`↴](#repoverlay-source)
* [`repoverlay source add`↴](#repoverlay-source-add)
* [`repoverlay source list`↴](#repoverlay-source-list)
* [`repoverlay source remove`↴](#repoverlay-source-remove)
* [`repoverlay completions`↴](#repoverlay-completions)

## `repoverlay`

Overlay config files into git repositories without committing them

**Usage:** `repoverlay [COMMAND]`

###### **Subcommands:**

* `apply` — Apply an overlay to a git repository
* `remove` — Remove applied overlay(s)
* `status` — Show the status of applied overlays
* `restore` — Restore overlays after git clean or other removal
* `update` — Update applied overlays from remote sources
* `create` — Create a new overlay from files in a repository
* `create-local` — Create a new overlay in a local directory
* `switch` — Switch to a different overlay (removes all existing overlays first)
* `cache` — Manage the overlay cache
* `list` — List available overlays from the overlay repository
* `sync` — Sync changes from an applied overlay back to the overlay repo
* `edit` — Edit an existing applied overlay (add files, remove files, or re-select interactively)
* `source` — Manage overlay sources (for multi-source configurations)
* `completions` — Generate shell completions



## `repoverlay apply`

Apply an overlay to a git repository

**Usage:** `repoverlay apply [OPTIONS] <SOURCE>`

###### **Arguments:**

* `<SOURCE>` — Path to overlay source directory OR GitHub URL

   Examples: ./my-overlay <https://github.com/owner/repo> <https://github.com/owner/repo/tree/main/overlays/rust>

###### **Options:**

* `-t`, `--target <TARGET>` — Target repository directory (defaults to current directory)
* `--copy` — Force copy mode instead of symlinks (default on Windows)
* `-n`, `--name <NAME>` — Override the overlay name (defaults to config name or directory name)
* `-r`, `--ref <REF>` — Git ref (branch, tag, or commit) to use (GitHub sources only)
* `--update` — Force update the cached repository before applying (GitHub sources only)
* `--force` — Overwrite existing files and re-apply same-name overlays
* `--skip-conflicts` — Skip conflicting files silently, continue with non-conflicting files
* `--merge` — Deep merge conflicting JSON files instead of failing
* `--from <SOURCE>` — Use a specific overlay source instead of priority order (multi-source configs only)
* `--dry-run` — Show what would be applied without making changes



## `repoverlay remove`

Remove applied overlay(s)

**Usage:** `repoverlay remove [OPTIONS] [NAME]`

###### **Arguments:**

* `<NAME>` — Name of the overlay to remove

###### **Options:**

* `-t`, `--target <TARGET>` — Target repository directory (defaults to current directory)
* `--all` — Remove all applied overlays
* `--dry-run` — Show what would be removed without making changes
* `-i`, `--interactive` — Interactive selection mode



## `repoverlay status`

Show the status of applied overlays

**Usage:** `repoverlay status [OPTIONS]`

###### **Options:**

* `-t`, `--target <TARGET>` — Target repository directory (defaults to current directory)
* `-n`, `--name <NAME>` — Show only a specific overlay



## `repoverlay restore`

Restore overlays after git clean or other removal

**Usage:** `repoverlay restore [OPTIONS]`

###### **Options:**

* `-t`, `--target <TARGET>` — Target repository directory (defaults to current directory)
* `--dry-run` — Show what would be restored without applying
* `--force` — Overwrite existing files during restore
* `--skip-conflicts` — Skip conflicting files silently during restore
* `--merge` — Deep merge conflicting JSON files instead of failing



## `repoverlay update`

Update applied overlays from remote sources

**Usage:** `repoverlay update [OPTIONS] [NAME]`

###### **Arguments:**

* `<NAME>` — Name of the overlay to update (updates all GitHub overlays if not specified)

###### **Options:**

* `-t`, `--target <TARGET>` — Target repository directory (defaults to current directory)
* `--dry-run` — Check for updates without applying them
* `--force` — Overwrite existing files during update
* `--skip-conflicts` — Skip conflicting files silently during update
* `--merge` — Deep merge conflicting JSON files instead of failing



## `repoverlay create`

Create a new overlay from files in a repository

Examples: repoverlay create my-overlay          # Detects org/repo from git remote repoverlay create org/repo/my-overlay # Explicit target

**Usage:** `repoverlay create [OPTIONS] <NAME>`

###### **Arguments:**

* `<NAME>` — Overlay name or full path (org/repo/name)

   Short form: `my-overlay` - detects org/repo from git remote Full form: `org/repo/name` - uses explicit target

###### **Options:**

* `-i`, `--include <INCLUDE>` — Include specific files or directories (can be specified multiple times)
* `-s`, `--source <SOURCE>` — Source repository to extract files from (defaults to current directory)
* `--dry-run` — Show what would be created without creating files
* `-y`, `--yes` — Skip interactive prompts, use defaults
* `-f`, `--force` — Force overwrite if overlay already exists



## `repoverlay create-local`

Create a new overlay in a local directory

Examples: repoverlay create-local ./output      # Write to local directory

**Usage:** `repoverlay create-local [OPTIONS] <OUTPUT>`

###### **Arguments:**

* `<OUTPUT>` — Output directory for the overlay

###### **Options:**

* `-i`, `--include <INCLUDE>` — Include specific files or directories (can be specified multiple times)
* `-s`, `--source <SOURCE>` — Source repository to extract files from (defaults to current directory)
* `--dry-run` — Show what would be created without creating files
* `-y`, `--yes` — Skip interactive prompts, use defaults
* `-f`, `--force` — Force overwrite if output already exists



## `repoverlay switch`

Switch to a different overlay (removes all existing overlays first)

**Usage:** `repoverlay switch [OPTIONS] <SOURCE>`

###### **Arguments:**

* `<SOURCE>` — Path to overlay source directory OR GitHub URL

###### **Options:**

* `-t`, `--target <TARGET>` — Target repository directory (defaults to current directory)
* `--copy` — Force copy mode instead of symlinks (default on Windows)
* `-n`, `--name <NAME>` — Override the overlay name
* `-r`, `--ref <REF>` — Git ref (branch, tag, or commit) to use (GitHub sources only)
* `--force` — Overwrite existing repo files when applying the new overlay
* `--skip-conflicts` — Skip conflicting repo files silently when applying the new overlay
* `--merge` — Deep merge conflicting JSON files instead of failing



## `repoverlay cache`

Manage the overlay cache

**Usage:** `repoverlay cache <COMMAND>`

###### **Subcommands:**

* `list` — List cached repositories
* `clear` — Clear all cached repositories
* `remove` — Remove a specific cached repository
* `path` — Show cache location



## `repoverlay cache list`

List cached repositories

**Usage:** `repoverlay cache list`



## `repoverlay cache clear`

Clear all cached repositories

**Usage:** `repoverlay cache clear [OPTIONS]`

###### **Options:**

* `-y`, `--yes` — Skip confirmation prompt



## `repoverlay cache remove`

Remove a specific cached repository

**Usage:** `repoverlay cache remove <REPO>`

###### **Arguments:**

* `<REPO>` — Repository to remove (format: owner/repo)



## `repoverlay cache path`

Show cache location

**Usage:** `repoverlay cache path`



## `repoverlay list`

List available overlays from the overlay repository

**Usage:** `repoverlay list [OPTIONS]`

###### **Options:**

* `-f`, `--filter <FILTER>` — Filter by target repository (format: org/repo)
* `--update` — Update overlay repo before listing



## `repoverlay sync`

Sync changes from an applied overlay back to the overlay repo

Examples: repoverlay sync my-overlay          # Detects org/repo from git remote repoverlay sync org/repo/my-overlay # Explicit target

**Usage:** `repoverlay sync [OPTIONS] <NAME>`

###### **Arguments:**

* `<NAME>` — Overlay name or full path (org/repo/name)

   Short form: `my-overlay` - detects org/repo from git remote Full form: `org/repo/name` - uses explicit values

###### **Options:**

* `-t`, `--target <TARGET>` — Target repository directory (defaults to current directory)
* `--dry-run` — Show what would be synced without making changes



## `repoverlay edit`

Edit an existing applied overlay (add files, remove files, or re-select interactively)

Examples: repoverlay edit my-overlay --add newfile.txt repoverlay edit my-overlay --remove oldfile.txt repoverlay edit my-overlay --add new.txt --remove old.txt repoverlay edit org/repo/my-overlay --interactive

**Usage:** `repoverlay edit [OPTIONS] <NAME>`

###### **Arguments:**

* `<NAME>` — Overlay name or full path (org/repo/name)

   Short form: `my-overlay` - detects org/repo from git remote Full form: `org/repo/name` - uses explicit values

###### **Options:**

* `-a`, `--add <FILE>` — Files to add to the overlay
* `-r`, `--remove <FILE>` — Files to remove from the overlay
* `-i`, `--interactive` — Re-run interactive file selection with current files pre-selected
* `-t`, `--target <TARGET>` — Target repository directory (defaults to current directory)
* `--dry-run` — Show what would change without making changes



## `repoverlay source`

Manage overlay sources (for multi-source configurations)

**Usage:** `repoverlay source <COMMAND>`

###### **Subcommands:**

* `add` — Add a new overlay source
* `list` — List configured overlay sources
* `remove` — Remove an overlay source



## `repoverlay source add`

Add a new overlay source

**Usage:** `repoverlay source add [OPTIONS] <URL>`

###### **Arguments:**

* `<URL>` — Git URL, GitHub shorthand (owner/repo), or GitHub username

###### **Options:**

* `--name <NAME>` — Name for this source (defaults to repo name)



## `repoverlay source list`

List configured overlay sources

**Usage:** `repoverlay source list`



## `repoverlay source remove`

Remove an overlay source

**Usage:** `repoverlay source remove <NAME>`

###### **Arguments:**

* `<NAME>` — Name of the source to remove



## `repoverlay completions`

Generate shell completions

**Usage:** `repoverlay completions <SHELL>`

###### **Arguments:**

* `<SHELL>` — Shell to generate completions for

  Possible values: `bash`, `elvish`, `fish`, `powershell`, `zsh`




<hr/>

<small><i>
    This document was generated automatically by
    <a href="https://crates.io/crates/clap-markdown"><code>clap-markdown</code></a>.
</i></small>

