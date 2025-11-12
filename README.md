# inro (印籠)

A minimalist, configuration-driven tool for installing and managing your favorite standalone binaries.

inro fetches binaries from sources like GitHub Releases and installs them into your home directory, requiring no admin rights. It's perfect for quickly bootstrapping your personal toolbox on any system.

## Notice

On Windows, inro requires that your account is [allowed to create symbolic links][windows-symlinks]. You can grant this permission in one of the following ways:

- Enable Developer Mode in the system settings.
- Run as an admin.

## Alternatives

- [marcosnils/bin][marcosnils/bin]
- [zyedidia/eget][zyedidia/eget]

When I started this project, I didn't really look into what was already out there. But so what? I enjoy reinventing the wheel, and I've also learned a lot from `bin`.
If you need a more general-purpose binary manager, I recommend using `bin`. If you want to quickly build your personal toolbox and gain more control, why not give `inro` a try?

## License

Copyright (c) Yangmoooo. Released under the MIT License. See [LICENSE][license] for details.

[windows-symlinks]: https://learn.microsoft.com/en-us/windows/security/threat-protection/security-policy-settings/create-symbolic-links
[license]: LICENSE
[marcosnils/bin]: https://github.com/marcosnils/bin
[zyedidia/eget]: https://github.com/zyedidia/eget
