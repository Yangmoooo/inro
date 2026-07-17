# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.9.1] - 2026-07-17

### Changed

- **Smaller Release Binaries**: Release builds now strip symbols, reducing artifact size without changing runtime behavior.

### Fixed

- **Efficient GitHub Release Lookup**: Explicit versions from `install`, `update`, and `import` now use GitHub's release-by-tag endpoint, while unversioned installs use the latest-release endpoint and only scan up to two 30-release pages when the latest release has no assets. `show` uses the same bounded pagination and stops once it has enough recent versions to display.

## [0.9.0] - 2026-07-17

### Added

- **Portable Package Sets**: `inro export` writes active packages with exact versions, and `inro import` validates and installs those exact versions through the normal install workflow for migration to another device.
- **Local Source Editor**: `inro source edit [name]` opens a staged copy of a hand-written file under `registry.d/` in the default editor and only replaces the live source after the merged registry validates successfully.

### Fixed

- **Reliable Install Receipts**: Installation receipts are now written before staged packages are activated. If a receipt cannot be created, the install fails without replacing the existing version.
- **Complete Async Downloads**: Package downloads are now flushed before extraction starts, preventing intermittent failures where installation could inspect a partially written asset.
- **Preserved Install Diagnostics**: Installation failures now retain their complete error chains, so verbose output reports underlying filesystem and remote causes instead of flattening them into generic messages.
- **Aligned Update Progress**: `inro update` now uses one shared progress layout for skipped, unchanged, and installed packages, keeping status rows consistently aligned.

## [0.8.1] - 2026-07-12

### Changed

- **Duplicate Package Arguments Are Rejected**: `install`, `update`, and `uninstall` now report an error when the same package is specified more than once, including conflicting forms such as `tool@1.0.0 tool@2.0.0`, instead of silently deduplicating the request.

### Fixed

- **Failure-Safe Package Operations**: Installing, updating, switching, unlinking, or uninstalling a package no longer leaves partially changed links, files, or manifest state when a later filesystem operation fails. Existing installations and links are restored where possible, failed removals remain recorded, and uninstalling an inactive version no longer removes links belonging to the active version.
- **Validated Source Updates**: `inro source update` now stages downloaded registries and validates both their TOML syntax and the merged registry before replacing cached sources. Invalid downloads leave the existing registry intact and cause the command to exit with an error.
- **Unlinked Packages Stay Unlinked**: `inro update` now skips installed packages with no active version instead of downloading an update and linking them again unexpectedly.
- **Doctor Exit Status**: `inro doctor` now exits with a non-zero status when it finds errors, so scripts and CI can reliably detect an unhealthy installation.
- **Accurate Clean Summary**: `inro clean` now reports the number of versions actually removed rather than counting removals that failed.
- **Registry Path Validation**: Package, source, version, binary, and link names that could escape their intended directories are now rejected with a clear error instead of being used as filesystem paths.

## [0.8.0] - 2026-06-29

### Added

- **Update Force & Version Pinning**: `inro update` now accepts `--force` (`-f`) to bypass the pin check, letting you update a pinned package without unpinning first (pin state is preserved). `inro update <pkg>@<version>` now respects the version specifier instead of ignoring it with a warning. `inro show` and `inro list` now display `[Pinned]` when a package is pinned, making pin state visible without running extra commands.
- **Verbosity Levels**: `-v` now shows detail lines (cause chains, intermediate steps) while `-vv` enables debug tracing output. Previously both flags produced identical output.

### Changed

- **`sources.list.d/` → `registry.d/`**: The user registry directory has been renamed from `sources.list.d` to `registry.d`. If you had hand-written files under the old path, move them over manually: `mv ~/.inro/sources.list.d ~/.inro/registry.d`. The old name is no longer read.

### Fixed

- **Doctor on Partial Registry Overrides**: `inro doctor` no longer treats fragment files like `registry/auto.toml` as broken. The per-file check is now syntax-only; schema validity is verified once against the merged registry, matching how `install`/`update` actually read it.

## [0.7.0] - 2026-06-11

**BREAKING CHANGE!** 0.7.0 reorganizes where inro keeps its files and rewrites the manifest schema. Old 0.6.x installations are not read — see migration notes below.

### Changed

- **Single-Root Layout**: All inro state now lives under one directory, `$INRO_HOME` (default `~/.inro/`), instead of being split between platform-specific config and data directories. Set `INRO_HOME` to relocate. Run `inro env` to see resolved paths.
- **Portable Manifest (schema v2)**: `PkgReceipt` now stores `install_subdir` and `bin_subpath` (relative to `pkgs_dir` and the install directory) instead of absolute `install_dir` / `bin_path` / `link_path`. Moving `$INRO_HOME` no longer invalidates the manifest, and `inro doctor --fix` re-points stale symlinks. Old (v1) manifests are rejected with a clear error.
- **Auto-Detected Asset Selectors**: When inro interactively picks a GitHub asset, the cached selector is now written to `$INRO_HOME/registry/auto.toml` (program-managed area) instead of `sources.list.d/local.toml` (user-authored area). Your hand-written files under `sources.list.d/` still take precedence.
- **Update Status**: `inro update` now uses a dim `=` marker followed by `(up to date)` for packages that were already at the latest version, distinguishing them from packages that were actually downloaded and installed (green `✓`).
- **`source list` Types**: The `Local` row is split into `Auto` (the program-maintained `auto.toml`) and `User` (entries under `sources.list.d/`).
- **Verbose Mode**: Use detail lines rather than progress bar in verbose mode.

### Added

- **`inro env`**: Prints `INRO_HOME` and every derived path (config, manifest, pkgs, registries, bin_dir). Useful for scripting and dotfiles setup.

### Upgrading from 0.6.x

Inro 0.7.0 reads only `$INRO_HOME` (default `~/.inro/`) and only accepts manifest schema v2. The old locations and the v1 manifest are not read.

**Replace the `inro` binary itself first.** A 0.6.x `inro` will write the wrong schema and refuse to manage symlinks pointing into the new `~/.inro/` tree. Install 0.7.0 to a location *outside* your inro-managed `bin_dir`, so the cleanup step below doesn't delete the binary you just installed.

```sh
# 1. Install inro 0.7.0 outside your inro bin_dir (default ~/.local/bin). Pick one:
cargo install --git https://github.com/Yangmoooo/inro.git
#   …puts it at ~/.cargo/bin/inro, separate from ~/.local/bin.
# Or grab a release binary from https://github.com/Yangmoooo/inro/releases and
# place it somewhere on $PATH that isn't your bin_dir, e.g. /usr/local/bin/inro.
inro --version   # confirm it reports 0.7.0

# 2. Back up your package list (the old manifest is still readable as JSON):
jq -r '.packages | keys[]' ~/.local/share/inro/inro-manifest.json > /tmp/pkgs.txt
# macOS:    ~/Library/Application\ Support/inro/inro-manifest.json
# Windows:  %LOCALAPPDATA%\inro\inro-manifest.json

# 3. Remove the old symlinks and old install directories:
jq -r '.packages[].versions[].binaries[].link_path' \
    ~/.local/share/inro/inro-manifest.json | xargs rm -f
rm -rf ~/.local/share/inro ~/.config/inro
# macOS:    rm -rf ~/Library/Application\ Support/inro
# Windows:  rmdir /s %APPDATA%\inro %LOCALAPPDATA%\inro

# 4. Reinstall packages under the new layout. If `inro` was in your old package
#    list, it will self-manage back into your bin_dir; you can then delete the
#    temporary copy from step 1.
inro install $(cat /tmp/pkgs.txt)
```

| OS | Old locations (no longer read) | New single root |
|---|---|---|
| Linux | `~/.config/inro/`, `~/.local/share/inro/` | `~/.inro/` |
| macOS | `~/Library/Application Support/inro/` | `~/.inro/` |
| Windows | `%APPDATA%\inro\`, `%LOCALAPPDATA%\inro\` | `%USERPROFILE%\.inro\` |

Do not copy the old `inro-manifest.json` into `~/.inro/manifest.json` — inro 0.7 will refuse it because the schema changed (paths are now portable rather than absolute). Follow the cleanup-and-reinstall steps above instead.

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
