# Inro ([印籠][inro])

A minimalist, configuration-driven tool for installing and managing your favorite command-line tools.

Inro fetches command-line tools from GitHub Releases or declarative direct download URLs and installs them under your home directory. It normally needs no admin rights; on Windows, creating the managed symlinks requires Developer Mode or an elevated shell.

## Installation

### Installer Script

On Linux or macOS:

```bash
curl --proto '=https' --tlsv1.2 -LsSf \
  https://raw.githubusercontent.com/Yangmoooo/inro/main/install.sh | sh
```

On Windows PowerShell:

```powershell
irm https://raw.githubusercontent.com/Yangmoooo/inro/main/install.ps1 | iex
```

The scripts download the latest release, verify its archive against `SHA256SUMS`, and place only
the `inro` binary (`inro.exe` on Windows) in the **current directory**. They do not run the binary
or modify PATH, configuration, registries, or the manifest. Existing files, directories, and
symbolic links with the same name are never overwritten.

To select a version or output directory:

```bash
curl --proto '=https' --tlsv1.2 -LsSf \
  https://raw.githubusercontent.com/Yangmoooo/inro/main/install.sh |
  sh -s -- --version 1.2.0 --to "$HOME/Downloads/inro"
```

```powershell
$installer = [scriptblock]::Create((irm https://raw.githubusercontent.com/Yangmoooo/inro/main/install.ps1))
& $installer -Version 1.2.0 -InstallDir "$HOME\Downloads\inro"
```

`INRO_VERSION` and `INRO_INSTALL_DIR` provide the same options; command-line arguments take
precedence. The scripts print the downloaded binary's path and suggested next steps.

You can run `./inro --help` (PowerShell: `.\inro.exe --help`), or move the binary to a directory
on PATH for manual use. To let inro manage itself, keep this copy outside the configured
`bin_dir` (default `~/.local/bin`) and run:

```bash
./inro source update
./inro install inro
```

On PowerShell, use `.\inro.exe` in place of `./inro`. These optional commands download a managed
copy, create its links and installation records, and require the usual network access and, on
Windows, symbolic-link permissions. After confirming that the managed binary is on PATH and
works, you can remove the original downloaded copy. Future updates use `inro update inro`.

### From Binaries

If a prebuilt archive is available for your platform on [GitHub Releases][releases], extract it and place the `inro` binary in your `PATH`. Releases currently provide Linux x86_64, Windows x86_64, and macOS arm64 builds; other Rust-supported targets can be built from source.

### From Source

If you have Rust installed, this builds the current `main` branch using the repository lockfile:

```bash
cargo install --locked --git https://github.com/Yangmoooo/inro.git
```

## Quick Start

Inro relies on a registry (source definitions) to know how to install packages.

##### Initialize/Update Sources

Fetches the default registry (and any custom ones).

```bash
inro source update
```

##### Search for Packages

Find tools available in the registry.

```bash
inro search ripgrep
```

##### Install a Tool

Downloads, extracts, and links the binary to your local bin directory.

```bash
inro install ripgrep
# Or install a specific version
inro install ripgrep@15.1.0
```

##### Manage

```bash
inro list                # List installed packages
inro update              # Update all packages
inro use ripgrep 15.0.0  # Switch version
inro unlink ripgrep      # Remove links while keeping installed files
inro relink ripgrep      # Apply registry binary definitions to the active version
inro uninstall ripgrep   # Remove a package
```

##### Move to Another Device

Export the active package versions, copy the file to the new device, initialize its sources, and
import the package set:

```bash
inro export --output inro-packages.txt
inro source update
inro import inro-packages.txt
```

Exported package sets contain exact active versions. Retained old versions, unlinked packages, pin
state, and local registry files are not included. Copy `registry.d/` separately before importing if
the package set depends on hand-written definitions. Every non-comment import line must include an
exact version in `<name>@<version>` form. Import still downloads from upstream, so that release and
its assets must remain available.

## Configuration

Inro keeps everything under a single root directory, `$INRO_HOME`. It defaults to `~/.inro/` on every platform; set `INRO_HOME` to relocate. Run `inro env` to see all resolved paths.

See [`config.example.toml`](config.example.toml) for the available settings and environment variable overrides.

```
$INRO_HOME/                      (default: ~/.inro/)
├── config.toml                  user configuration
├── manifest.json                installed packages state
├── registry.d/                  your hand-written registry overrides
│   └── *.toml
├── registry/                    inro-maintained
│   ├── 00-default.toml          fetched by `inro source update`
│   └── auto.toml                auto-detected asset selectors
└── pkgs/                        installed package versions
```

Anything under `registry.d/` is yours to author — its entries take precedence over `registry/` on load, so you can override a definition that inro pulled from upstream or learned automatically. Automated source updates never write there. Inro itself changes these files only when you explicitly run `source edit`; editing them directly remains supported.

Use `inro source edit` to create or edit `registry.d/local.toml`, or
`inro source edit <name>` for another hand-written registry file. Edits are staged and only replace
the live file after the merged registry validates successfully. Inro uses `$VISUAL` or `$EDITOR`
when set; the command must wait until editing finishes (for example, `code --wait`). Without either
variable it falls back to `vi` on Linux/macOS, or `edit.exe` followed by `notepad.exe` on Windows.

### Binary Links

The package's `bin` list selects which installed executables are linked into `bin_dir`. Each `name`
is a filename to find inside the installed package; `link` optionally gives it a different command
name. Unlisted files remain in the installation directory.

Local `bin` arrays replace the entire upstream list, so include every command you want linked.
For example, after installing `uv`, run `inro source edit` and add this to
`$INRO_HOME/registry.d/local.toml`:

```toml
[uv]
bin = [
  { name = "uv" },
  { name = "uvx", link = "uv-tool" },
]
```

The package's download configuration is inherited from the registry. Apply the new links with:

```bash
inro relink uv
```

This keeps `uv` linked, exposes `uvx` as `uv-tool`, and removes the old `uvx` link if it still belongs
to this version. Editing the registry alone does not change existing links. Omitting `bin`, or
setting `bin = []`, uses the package name as the default binary name; use `inro unlink uv` to
temporarily remove all of the package's managed links.

`relink` only operates on the active installed version and uses the merged registry already on
disk. It adds, removes, renames, or repairs links, updates the manifest and installation receipt,
and preserves the version, source, installation time, and pin state. It does not download files,
refresh registry sources, or apply changes to download URLs. If the package is unlinked, first
select an installed version with `inro use <package> <version>`, then run `inro relink <package>`.
`use` and `doctor --fix` continue to restore links from installation records without applying
registry edits.

Missing binaries, duplicate link names, or conflicting destinations are rejected before links are
changed. New links cannot replace user files or links belonging to other packages. Obsolete links
replaced by user files or another version or package are left alone. If a later link or record
update fails, inro attempts to restore the previous links and records and reports any rollback
failure. When a requested executable is absent from the installed files, install a version that
includes it before retrying.

### Asset Selectors

GitHub packages can define platform-specific asset selectors when automatic asset discovery is ambiguous.

```toml
[aria2.remote.github]
repo = "aria2/aria2"

[aria2.remote.github.asset]
"linux-x86_64" = "aria2-*.tar.xz"
"windows-x86_64" = ["aria2", "win", "64bit", "zip"]
```

- String selectors are minimal glob patterns matched against the full asset name. `*` matches any number of characters and `?` matches one character.
- Array selectors are all-of tokens. Every token must appear in the asset name.
- When inro picks an asset interactively, it remembers the choice in `$INRO_HOME/registry/auto.toml`. If that rule no longer matches (for example, upstream renamed its assets), install/update retries the same release using the upstream rule, or platform matching when no rule is configured. Ambiguous replacements require an interactive choice; non-interactive runs report how to resolve the ambiguity. Hand-written rules remain strict, including rules in `registry.d/` that repeat an automatic choice.
- Obsolete automatic rules are removed or replaced only after installation succeeds, and only for the affected package and platform. Network/download errors, failed installations, and cancelled selections leave the remembered rules intact. Automatic asset learning never writes to your files under `registry.d/`.

To reset remembered choices manually, use:

```sh
inro clean --assets terraform           # Clear this package's choices on all platforms
inro clean --assets                    # Clear all remembered choices
inro clean --assets terraform --dry-run # Preview the choices that would be cleared
```

Asset cleanup only changes automatic selections. It preserves installed versions, upstream registry
files, and hand-written overrides. Plain `inro clean` continues to clean unused package versions.

### Direct Downloads

For tools whose maintainers publish archives outside GitHub Releases, a package can declare exact
version-to-URL mappings:

```toml
[sqlite]

[sqlite.remote.direct."3.53.4"]
"linux-x86_64" = "https://www.sqlite.org/2026/sqlite-tools-linux-x64-3530400.zip"
"macos-aarch64" = "https://www.sqlite.org/2026/sqlite-tools-osx-arm64-3530400.zip"
"windows-x86_64" = "https://www.sqlite.org/2026/sqlite-tools-win-x64-3530400.zip"

[[sqlite.bin]]
name = "sqlite3"
```

The quoted table key is the package version, so a version cannot be declared without its platform
URLs. Binary lookup still uses the normal package-level `[[package.bin]]` entries. The current
platform must have an exact `<os>-<arch>` key.

Direct remotes are deliberately declarative: inro does not scrape download pages, discover new
versions, infer a latest version, or attach release dates. With one configured version,
`inro install sqlite` uses that version. With multiple configured versions,
install requires an exact specification such as `sqlite@3.53.4`. Unversioned updates (`inro update`
or `inro update sqlite`) skip all direct packages with a reminder to choose a version, without treating
the skip as a failure or interrupting other updates. Use `inro update sqlite@3.53.4` to select an
exact version; `--force` only bypasses pinning and does not choose a version. Invalid versions and
download failures still fail normally. Exact versions in import files work the same way.
`inro show sqlite` lists the versions declared in the registry without
network access. Additional versions can be merged in through a file under `registry.d/`.
Removing a version from the registry does not remove its installed files or installation record:
it can still be used, selected with `use`, or uninstalled. Downloading that version again (including
importing an exported package set on another machine) requires restoring its URL definition.
Direct remotes currently do not perform checksum verification, so the registry maintainer is
responsible for keeping the declared URLs trustworthy.

## Compatibility

Starting with 1.0, inro follows Semantic Versioning for its documented interfaces. Within the 1.x
series:

- Documented commands, flags, package specifications (`name` and `name@version`), config keys, and
  environment variables remain compatible.
- GitHub registry definitions continue to accept literal and platform-specific binary names, plus
  the documented string and token-list asset selectors. New optional fields may be added.
- Direct registry definitions continue to use quoted version table keys with platform-specific
  HTTP(S) URLs and the normal package-level binary definitions.
- `$INRO_HOME` keeps the documented ownership split between `registry.d/` and inro-managed files.
  Manifest schema v2 and current install receipts remain readable; an incompatible state change
  requires a schema change and explicit migration or reinstall guidance.
- Exported package sets remain importable and keep the documented exact `name@version` line format.

Human-readable progress and diagnostic wording are not a scripting interface. Scripts should use
the process success/failure status and `inro export` rather than parsing ordinary terminal output.
Prebuilt releases currently support Linux x86_64, Windows x86_64, and macOS arm64; unsupported
Rust targets can still be built from source.

## Notice

On Windows, inro requires that your account is [allowed to create symbolic links][windows-symlinks]. You can grant this permission in one of the following ways:

- Enable Developer Mode in the system settings.
- Run as an admin.

## Alternatives

When I started this project, I didn't really look into existing solutions. But so what? I enjoy reinventing the wheel, and it was a great learning opportunity for me.

For a general-purpose binary manager/installer, recommend:

- [marcosnils/bin][marcosnils/bin]
- [zyedidia/eget][zyedidia/eget]

For powerful features like environment management and so on, consider:

- [jdx/mise][jdx/mise]
- [x-cmd/x-cmd][x-cmd/x-cmd]

If you want a balanced middle ground, give `inro` a try. It's lightweight, focused, and non-intrusive, making it ideal for building a personal toolbox without the overhead.

A huge thanks to these projects—I've learned so much from them.

## License

Copyright (c) Yangmoooo. Released under the MIT License. See [LICENSE][license] for details.

[inro]: https://en.wiktionary.org/wiki/%E5%8D%B0%E7%B1%A0
[releases]: https://github.com/Yangmoooo/inro/releases
[windows-symlinks]: https://learn.microsoft.com/en-us/windows/security/threat-protection/security-policy-settings/create-symbolic-links
[license]: LICENSE
[marcosnils/bin]: https://github.com/marcosnils/bin
[zyedidia/eget]: https://github.com/zyedidia/eget
[jdx/mise]: https://github.com/jdx/mise
[x-cmd/x-cmd]: https://github.com/x-cmd/x-cmd
