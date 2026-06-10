# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

**BREAKING CHANGE!** 0.7.0 reorganizes where inro keeps its files. Read the migration notes below before upgrading.

### Changed

- **Single-Root Layout**: All inro state now lives under one directory, `$INRO_HOME` (default `~/.inro/`), instead of being split between platform-specific config and data directories. Set `INRO_HOME` to relocate. Run `inro env` to see resolved paths. Inro does not migrate old state automatically — see migration notes.
- **Auto-Detected Asset Selectors**: When inro interactively picks a GitHub asset, the cached selector is now written to `$INRO_HOME/registry/auto.toml` (program-managed area) instead of `sources.list.d/local.toml` (user-authored area). Your hand-written files under `sources.list.d/` still take precedence.
- **Update Status**: `inro update` now uses a dim `=` marker followed by `(up to date)` for packages that were already at the latest version, distinguishing them from packages that were actually downloaded and installed (green `✓`).
- **`source list` Types**: The `Local` row is split into `Auto` (the program-maintained `auto.toml`) and `User` (entries under `sources.list.d/`).
- **Verbose Mode**: Use detail lines rather than progress bar in verbose mode.

### Added

- **`inro env`**: Prints `INRO_HOME` and every derived path (config, manifest, pkgs, registries, bin_dir). Useful for scripting and dotfiles setup.

### Migration

Inro 0.7.0 reads only `$INRO_HOME`. The old locations are no longer touched. To carry your state over from 0.6.x:

| OS | Old config | Old data | New (everything) |
|---|---|---|---|
| Linux | `~/.config/inro/` | `~/.local/share/inro/` | `~/.inro/` |
| macOS | `~/Library/Application Support/inro/` | `~/Library/Application Support/inro/` | `~/.inro/` |
| Windows | `%APPDATA%\inro\` | `%LOCALAPPDATA%\inro\` | `%USERPROFILE%\.inro\` |

Example for Linux/macOS — merge old config and data into `~/.inro/`, then rename `inro-manifest.json`:

```sh
mkdir -p ~/.inro
# adjust paths for macOS if needed
mv ~/.config/inro/config.toml         ~/.inro/config.toml
mv ~/.local/share/inro/inro-manifest.json ~/.inro/manifest.json
mv ~/.local/share/inro/pkgs           ~/.inro/pkgs
mv ~/.local/share/inro/sources.list.d ~/.inro/registry
mv ~/.config/inro/sources.list.d      ~/.inro/sources.list.d
```

Your existing `sources.list.d/local.toml` (if any) keeps working — inro 0.7.0 just won't write to it. Once you've upgraded, delete it to let inro re-learn selectors into `registry/auto.toml`, or keep it as a hand-managed override.

## [0.6.2] - 2026-04-26

### Fixed

- **No More Clobbering Foreign Files**: Installing or updating a package whose binary name collides with an existing file in your `bin_dir` (for example, an apt-/brew-/cargo-installed `rg`) used to silently delete that file and replace it with inro's own symlink. inro now refuses to touch anything that is not already a symlink it manages, and tells you the conflicting path so you can decide what to do.
- **Failed Installs Don't Damage Existing Versions**: `inro install` now stages every install in a sibling directory and only swaps it into place after the download, extraction, and binary discovery all succeed. A network blip or bad archive no longer leaves a half-built version directory on disk, and reinstalling the currently-active version is safe — a failure halfway through can no longer leave you with a dangling symlink and a manifest entry pointing at nothing.

## [0.6.1] - 2026-04-26

### Fixed

- **Tar Symlinks**: Tarballs containing ordinary relative symlinks were rejected as if they tried to escape the destination. They now extract correctly while real traversal attempts and absolute targets are still blocked.
- **Network Hangs**: A slow DNS resolver, unreachable host, or stalled server could keep `install`, `update`, or `source update` waiting forever. All HTTP requests now enforce sensible connect and read timeouts.
- **GitHub Rate Limits**: Hitting the API limit used to return a generic HTTP error. inro now reports when the limit resets and suggests setting `INRO_GITHUB_TOKEN` / `GITHUB_TOKEN` (or, if one is already set, points at the token).
- **Concurrent Runs**: Two `inro` invocations at the same time could silently corrupt shared state. The second one now waits for the first to finish, with a brief notice while it waits.
- **Older Versions Reachable**: `inro show` and `inro install pkg@<tag>` only saw the latest 20 releases, hiding older versions on fast-moving projects. They now look at the latest 100.
- **Empty Binary Crash**: Installing a package whose `[bin]` entries were all filtered out for the current platform crashed; you now get a clear error naming the platform.
- **Deterministic Binary Pick**: When several extracted files share the requested binary name, inro picks based on file signature and location instead of whichever the filesystem yielded first.
- **Clearer Setup Errors**: Failure to create the configured `bin_dir` is now reported directly, instead of surfacing later as a confusing symlink error.
- **Silent Skips**: Skipped zip entries with suspicious paths, and failures to write the per-install receipt, now produce warnings instead of disappearing.

## [0.6.0] - 2026-04-26

**BREAKING CHANGE!** Please do not continue using the outdated 0.5.x version; upgrade to the latest version as soon as possible according to the instructions.

### Changed

- **Asset Selectors**: GitHub `remote.github.asset` strings are now glob patterns matched against the full asset name, and arrays are supported as all-of token selectors. Old local substring-based overrides should be removed from `~/.config/inro/sources.list.d/local.toml` so inro can regenerate them.

### Fixed

- **Binary Detect Error**: Returns an error instead of a fallback when a suitable executable file cannot be found in the asset.

## [0.5.1] - 2026-04-24

### Fixed

- **macOS Release Binary**: Statically link `liblzma` for `.tar.xz` support so the macOS release artifact no longer depends on Homebrew's `xz` library path.

## [0.5.0] - 2026-04-24

### Added

- **Interactive Asset Selection**: Prompt users to choose from multiple GitHub release assets and persist explicit choices to local registry overrides.
- **macOS Support**: Add CI and release artifact support for macOS aarch64.
- **Mach-O Binaries**: Support installing standalone macOS Mach-O binaries without treating them as archives.
- **Verbose Diagnostics**: Improve `-v` output with error cause chains and GitHub asset matching details.

### Changed

- **Asset Discovery**: Fall back to supported release assets when platform heuristics cannot find an OS/Arch match.
- **Local Overrides**: Write selected GitHub assets using compact dotted TOML tables for more readable local configuration.

### Fixed

- **Remote Errors**: Preserve specific GitHub error messages instead of replacing them with generic upstream fetch failures.
- **Asset Persistence**: Make local asset selection write-back safer across platforms and preserve upstream package definitions when merging overrides.

## [0.4.2] - 2026-04-22

### Fixed

- **Symbolic Link Handling**: Fix issues with symbolic links in certain environments.

## [0.4.1] - 2026-03-24

### Fixed

- **Binary Not Found**: Fix a phantom lookup in assets.

## [0.4.0] - 2026-03-13

### Added

- **Exact Platform Matching**: Allow to specify exact OS/Arch matches in asset discovery, reducing false positives.
- **Enhanced Source**: `source list` command can now display which remote registries need to be updated in the local cache. `source enable/disable` commands control whether a specific remote registry is enabled.

### Fixed

- **HTTP Error Handling**: Handle HTTP errors in ordinary downloading.
- **Clean Thoroughly**: `clean` command now correctly updates the manifest when removing specific versions, preventing orphaned entries.

### Security

- **Safe Path Resolution**: prevent directory traversal in TAR extraction.

### Removed

- **Simplify Config**: `timeout` is no longer used. `proxy` and `token` are only configured via environment variables.

## [0.3.0] - 2026-03-04

### Added

- **Pin Version**: `pin` command for preventing packages from being auto-updated.
- **Spot Risks**: `doctor` command for find hidden dangers.

### Changed

- **Rename `info`**: Rename command `info` to `show`.

## [0.2.1] - 2026-02-03

### Fixed

- **Faster extract**: Significantly faster when extracting large files in `.zip` and `.tar`

## [0.2.0] - 2026-02-02

### Added

- **Async Download**: `install` and `update` commands support async now!
- **Progress Bar**: There are progress bars when downloading.

## [0.1.0] - 2025-12-31

### Added

- **Initial Release**: First public release of `inro`.
- **Core Commands**:
  - `install`: Support for latest or specific versions (`pkg@ver`).
  - `uninstall`: Support for removing specific versions or all versions.
  - `update`: Upgrade packages to the latest suitable version.
  - `source`: Manage upstream registries.
  - `use`: Switch between installed versions instantly.
  - `list`, `info`, `clean`, `unlink`.
- **Multi-Version Management**: Ability to install and manage multiple versions of the same package side-by-side.
- **Smart Discovery**: Automatically detects platform-specific assets (OS/Arch) from GitHub Releases.
- **Archive Support**: Native extraction for `.tar.gz`, `.tar.xz`, `.tar.bz2`, `.zip`, `.7z`, and standalone binaries.
- **Registry System**: Configuration-driven package definitions supporting local overrides and remote updates.
- **Shell Integration**: Built-in generator for Man pages and shell completions (Bash, Zsh, Fish, PowerShell).
