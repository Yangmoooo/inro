# Inro ([印籠][inro])

A minimalist, configuration-driven tool for installing and managing your favorite command-line tools.

Inro fetches apps from sources like GitHub Releases and installs them into your home directory, requiring no admin rights. It's perfect for quickly bootstrapping your personal toolbox on any system.

## Installation

### From Binaries

Download the latest archive for your platform from [GitHub Releases][releases], extract it, and place the `inro` binary in your `PATH`.

### From Source

If you have Rust installed:

```bash
cargo install --git https://github.com/Yangmoooo/inro.git
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

## Configuration

Inro is configuration-driven. You can override defaults or add custom sources.

- config file at
  - Linux: `~/.config/inro/config.toml`
  - Windows: `%APPDATA%\inro\config.toml`
- sources file at
  - Linux: `~/.config/inro/sources.list.d/foobar.toml`
  - Windows: `%APPDATA%\inro\sources.list.d\foobar.toml`

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
