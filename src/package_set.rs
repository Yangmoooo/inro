use anyhow::{Context, Result, bail};

use crate::manifest::Manifest;
use crate::utils::{ensure_unique_package_args, parse_package_version, validate_path_component};

pub struct RenderedPackageSet {
    pub contents: String,
    pub skipped_unlinked: usize,
}

pub fn render(manifest: &Manifest) -> RenderedPackageSet {
    let mut packages = Vec::new();
    let mut skipped_unlinked = 0usize;
    for (name, state) in &manifest.pkgs {
        if let Some(version) = &state.current_version {
            packages.push((name.as_str(), version.as_str()));
        } else {
            skipped_unlinked += 1;
        }
    }
    packages.sort_unstable_by_key(|(name, _)| *name);
    let specs: Vec<String> =
        packages.into_iter().map(|(name, version)| format!("{name}@{version}")).collect();
    let contents = if specs.is_empty() { String::new() } else { format!("{}\n", specs.join("\n")) };

    RenderedPackageSet { contents, skipped_unlinked }
}

pub fn parse(contents: &str) -> Result<Vec<String>> {
    let specs: Vec<String> = contents
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(str::to_string)
        .collect();
    if specs.is_empty() {
        bail!("Package set contains no package specifications");
    }

    for spec in &specs {
        let (name, version) = parse_package_version(spec);
        if name.is_empty() || version == Some("") {
            bail!("Invalid package specification '{spec}': name and version must not be empty");
        }
        if version.is_none() {
            bail!("Invalid package specification '{spec}': an exact version is required");
        }
        validate_path_component(name, "package name")
            .with_context(|| format!("Invalid package specification '{spec}'"))?;
    }
    ensure_unique_package_args(&specs)?;

    Ok(specs)
}
