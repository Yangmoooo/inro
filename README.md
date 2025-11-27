# Inro ([印籠][inro])

A minimalist, configuration-driven tool for installing and managing your favorite command-line tools.

Inro fetches apps from sources like GitHub Releases and installs them into your home directory, requiring no admin rights. It's perfect for quickly bootstrapping your personal toolbox on any system.

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
[windows-symlinks]: https://learn.microsoft.com/en-us/windows/security/threat-protection/security-policy-settings/create-symbolic-links
[license]: LICENSE
[marcosnils/bin]: https://github.com/marcosnils/bin
[zyedidia/eget]: https://github.com/zyedidia/eget
[jdx/mise]: https://github.com/jdx/mise
[x-cmd/x-cmd]: https://github.com/x-cmd/x-cmd
