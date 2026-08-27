use core::slice;
use std::env::consts;

#[derive(Debug)]
pub struct PlatformInfo {
    pub os: &'static str,
    pub arch: &'static str,
}

impl PlatformInfo {
    pub fn current() -> Self { Self { os: consts::OS, arch: consts::ARCH } }

    pub fn key(&self) -> String { format!("{}-{}", self.os, self.arch) }

    pub fn os_aliases(&self) -> &[&str] {
        match self.os {
            "linux" => &["linux"],
            "macos" => &["darwin", "apple", "macos", "osx"],
            "windows" => &["windows", "-win", "_win"],
            _ => slice::from_ref(&self.os),
        }
    }

    pub fn arch_aliases(&self) -> &[&str] {
        match self.arch {
            "x86_64" => &["x86_64", "amd64", "x64"],
            "aarch64" => &["aarch64", "arm64"],
            "x86" => &["x86", "i386", "i686", "386"],
            _ => slice::from_ref(&self.arch),
        }
    }

    /// Check whether an asset name identifies this platform using either the
    /// usual independent OS/architecture markers or a compound marker such
    /// as `win64`.
    pub fn matches_asset_name(&self, name: &str) -> bool {
        let name = name.to_ascii_lowercase();
        let os_match = self.os_aliases().iter().any(|alias| name.contains(alias));
        let arch_match = self.arch_aliases().iter().any(|alias| name.contains(alias));

        if os_match && arch_match {
            return true;
        }

        self.compound_asset_aliases().iter().any(|alias| {
            name.split(|ch: char| !ch.is_ascii_alphanumeric()).any(|part| part == *alias)
        })
    }

    fn compound_asset_aliases(&self) -> &[&str] {
        match (self.os, self.arch) {
            ("windows", "x86_64") => &["win64"],
            ("windows", "x86") => &["win32"],
            _ => &[],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn platform_key_format() {
        let platform = PlatformInfo { os: "linux", arch: "x86_64" };
        assert_eq!(platform.key(), "linux-x86_64");

        let platform = PlatformInfo { os: "windows", arch: "aarch64" };
        assert_eq!(platform.key(), "windows-aarch64");
    }

    #[test]
    fn os_aliases_linux() {
        let platform = PlatformInfo { os: "linux", arch: "x86_64" };
        let aliases = platform.os_aliases();
        assert!(aliases.contains(&"linux"));
    }

    #[test]
    fn os_aliases_macos() {
        let platform = PlatformInfo { os: "macos", arch: "aarch64" };
        let aliases = platform.os_aliases();
        assert!(aliases.contains(&"darwin"));
        assert!(aliases.contains(&"apple"));
        assert!(aliases.contains(&"macos"));
        assert!(aliases.contains(&"osx"));
    }

    #[test]
    fn os_aliases_windows() {
        let platform = PlatformInfo { os: "windows", arch: "x86_64" };
        let aliases = platform.os_aliases();
        assert!(aliases.contains(&"windows"));
        assert!(aliases.contains(&"-win"));
        assert!(aliases.contains(&"_win"));
    }

    #[test]
    fn arch_aliases_x86_64() {
        let platform = PlatformInfo { os: "linux", arch: "x86_64" };
        let aliases = platform.arch_aliases();
        assert!(aliases.contains(&"x86_64"));
        assert!(aliases.contains(&"amd64"));
        assert!(aliases.contains(&"x64"));
    }

    #[test]
    fn arch_aliases_aarch64() {
        let platform = PlatformInfo { os: "linux", arch: "aarch64" };
        let aliases = platform.arch_aliases();
        assert!(aliases.contains(&"aarch64"));
        assert!(aliases.contains(&"arm64"));
    }

    #[test]
    fn arch_aliases_x86() {
        let platform = PlatformInfo { os: "linux", arch: "x86" };
        let aliases = platform.arch_aliases();
        assert!(aliases.contains(&"x86"));
        assert!(aliases.contains(&"i386"));
        assert!(aliases.contains(&"i686"));
        assert!(aliases.contains(&"386"));
    }

    #[test]
    fn aliases_unknown_fallback() {
        let platform = PlatformInfo { os: "freebsd", arch: "riscv64" };
        assert!(platform.os_aliases().contains(&"freebsd"));
        assert!(platform.arch_aliases().contains(&"riscv64"));
    }

    #[test]
    fn asset_names_match_windows_compound_architectures() {
        let x64 = PlatformInfo { os: "windows", arch: "x86_64" };
        assert!(x64.matches_asset_name("upx-5.2.0-win64.zip"));
        assert!(!x64.matches_asset_name("upx-5.2.0-win32.zip"));

        let x86 = PlatformInfo { os: "windows", arch: "x86" };
        assert!(x86.matches_asset_name("upx-5.2.0-win32.zip"));
        assert!(!x86.matches_asset_name("upx-5.2.0-win64.zip"));
    }

    #[test]
    fn compound_asset_aliases_require_a_token_boundary() {
        let x64 = PlatformInfo { os: "windows", arch: "x86_64" };
        assert!(!x64.matches_asset_name("upx-5.2.0-darwin64.zip"));
        assert!(!x64.matches_asset_name("upx-5.2.0-win64msvc.zip"));
    }
}
