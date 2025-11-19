use core::slice;
use std::env::consts;

#[derive(Debug)]
pub struct PlatformInfo {
    pub os: &'static str,
    pub arch: &'static str,
}

impl PlatformInfo {
    pub fn current() -> Self {
        Self {
            os: consts::OS,
            arch: consts::ARCH,
        }
    }
    pub fn key(&self) -> String {
        format!("{}-{}", self.os, self.arch)
    }

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
}
