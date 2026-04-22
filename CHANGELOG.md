# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
