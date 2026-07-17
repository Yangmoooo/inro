# Inro ([印籠][inro])

A minimalist, configuration-driven tool for installing and managing your favorite command-line tools.

Inro fetches command-line tools from GitHub Releases and installs them under your home directory. It normally needs no admin rights; on Windows, creating the managed symlinks requires Developer Mode or an elevated shell.

## Installation

You can install inro using either method below. Inro can later manage its own updates, but the bootstrap executable must remain outside the configured `bin_dir`: run `inro source update` and `inro install inro`, confirm that `bin_dir` is on your `PATH` and the managed binary is being invoked, then delete the bootstrap copy.

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

> Upgrading from 0.6.x? Your old installations under `~/.local/share/inro/` and `~/.config/inro/` (or `~/Library/Application Support/inro/` on macOS) are no longer read. See [CHANGELOG][changelog] for the cleanup-and-reinstall path.

[changelog]: CHANGELOG.md

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
- When inro picks an asset interactively, it caches the choice in `$INRO_HOME/registry/auto.toml`. If a learned selector no longer matches (e.g. upstream renamed its assets), delete that file and re-run the install/update — inro will re-learn. Automatic asset learning never writes to your files under `registry.d/`.

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
