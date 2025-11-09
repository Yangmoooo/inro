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
}
