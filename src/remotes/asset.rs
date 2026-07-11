use std::fmt;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum AssetSelector {
    Glob(String),
    Tokens(Vec<String>),
}

impl fmt::Display for AssetSelector {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AssetSelector::Glob(pattern) => write!(f, "{pattern}"),
            AssetSelector::Tokens(tokens) => write!(f, "{}", tokens.join(", ")),
        }
    }
}

const BLOCK_EXTENSIONS: &[&str] = &[
    // Checksum
    ".sha256",
    ".sha256sum",
    ".md5",
    ".asc",
    ".sig",
    // Plain
    ".txt",
    ".md",
    // Data
    ".xml",
    ".json",
    ".yml",
    ".yaml",
    ".toml",
    // Installer
    ".deb",
    ".rpm",
    ".msi",
    ".pkg",
    ".dmg",
];
const ALLOW_EXTENSIONS: &[&str] = &[
    ".tar.gz", ".tgz", //
    ".tar.xz", ".txz", //
    ".tar.bz2", ".tbz", //
    ".7z",  //
    ".zip", //
    ".exe", //
];

pub fn is_ignored_format(name: &str) -> bool {
    BLOCK_EXTENSIONS.iter().any(|ext| name.ends_with(ext))
}
pub fn is_supported_format(name: &str) -> bool {
    if ALLOW_EXTENSIONS.iter().any(|ext| name.ends_with(ext)) {
        return true;
    }
    if !name.contains('.') {
        return true;
    }
    false // An elf like xxx-v0.1.0-linux-x86_64 need to be specified in registry
}

/// Derive a version-agnostic asset selector from an asset filename.
///
/// Given an asset like `ripgrep-15.1.0-x86_64-apple-darwin.tar.gz` and
/// version tag `v15.1.0`, produces a glob like
/// `ripgrep-*-x86_64-apple-darwin.tar.gz` when possible.
pub fn derive_asset_selector(asset_name: &str, version_tag: &str) -> AssetSelector {
    derive_asset_selector_from_assets(asset_name, version_tag, &[asset_name.to_string()])
}

/// Derive a version-agnostic selector using the whole release asset list to
/// prefer selectors that uniquely identify the selected asset.
pub fn derive_asset_selector_from_assets(
    asset_name: &str,
    version_tag: &str,
    all_asset_names: &[String],
) -> AssetSelector {
    let mut candidates: Vec<AssetSelector> = asset_glob_candidates(asset_name, version_tag)
        .into_iter()
        .map(AssetSelector::Glob)
        .collect();
    candidates.push(AssetSelector::Glob(glob_escape(asset_name)));

    let tokens = stable_asset_tokens(asset_name, version_tag);
    if !tokens.is_empty() {
        candidates.push(AssetSelector::Tokens(tokens));
    }

    let mut unique_candidates = Vec::new();
    for candidate in candidates {
        if !unique_candidates.contains(&candidate) {
            unique_candidates.push(candidate);
        }
    }

    let mut scored_candidates: Vec<_> = unique_candidates
        .into_iter()
        .map(|selector| {
            let score = score_asset_selector(&selector, asset_name, version_tag, all_asset_names);
            (selector, score)
        })
        .collect();
    scored_candidates.sort_by_key(|b| std::cmp::Reverse(b.1));

    scored_candidates
        .into_iter()
        .find_map(|(selector, score)| (score > i32::MIN).then_some(selector))
        .unwrap_or_else(|| AssetSelector::Glob(glob_escape(asset_name)))
}

pub fn asset_matches_selector(asset_name: &str, selector: &AssetSelector) -> bool {
    match selector {
        AssetSelector::Glob(pattern) => glob_match(pattern, asset_name),
        AssetSelector::Tokens(tokens) => {
            !tokens.is_empty() && tokens.iter().all(|token| asset_name.contains(token))
        }
    }
}

fn asset_glob_candidates(asset_name: &str, version_tag: &str) -> Vec<String> {
    let mut candidates = Vec::new();
    for version in version_variants(version_tag) {
        for (start, _) in asset_name.match_indices(&version) {
            let end = start + version.len();
            let before = clean_asset_glob_prefix(&asset_name[..start]);
            let after = glob_escape(&asset_name[end..]);
            candidates.push(format!("{before}*{after}"));
        }
    }
    candidates.sort();
    candidates.dedup();
    candidates
}

fn version_variants(version_tag: &str) -> Vec<String> {
    let mut variants = vec![version_tag.to_string()];
    if let Some(bare) = version_tag.strip_prefix('v')
        && !bare.is_empty()
    {
        variants.push(bare.to_string());
    } else if is_plain_version_like(version_tag) {
        variants.push(format!("v{version_tag}"));
    }
    variants.extend(version_like_fragments(version_tag));
    variants.sort_by_key(|s| std::cmp::Reverse(s.len()));
    variants.dedup();
    variants
}

fn version_like_fragments(s: &str) -> Vec<String> {
    let mut fragments = Vec::new();
    let bytes = s.as_bytes();
    let mut start = None;

    for (idx, byte) in bytes.iter().enumerate() {
        let is_version_char = byte.is_ascii_digit()
            || (*byte == b'.' && start.is_some())
            || (*byte == b'-' && start.is_some())
            || (*byte == b'_' && start.is_some());

        if byte.is_ascii_digit() {
            if start.is_none() {
                start = Some(idx);
            }
        } else if !is_version_char && let Some(fragment_start) = start.take() {
            push_version_like_fragment(&mut fragments, &s[fragment_start..idx]);
        }
    }

    if let Some(fragment_start) = start {
        push_version_like_fragment(&mut fragments, &s[fragment_start..]);
    }

    fragments
}

fn push_version_like_fragment(fragments: &mut Vec<String>, raw: &str) {
    let fragment = raw.trim_matches(['-', '_', '.']);
    if is_plain_version_like(fragment) {
        fragments.push(fragment.to_string());
    }
}

fn is_plain_version_like(s: &str) -> bool {
    let mut dot_count = 0usize;
    let mut digit_count = 0usize;

    for ch in s.chars() {
        if ch.is_ascii_digit() {
            digit_count += 1;
        } else if ch == '.' {
            dot_count += 1;
        } else if ch == '-' || ch == '_' || ch.is_ascii_alphabetic() {
            // allow simple pre-release/build-ish suffixes such as 1.2.3-beta.1
        } else {
            return false;
        }
    }

    digit_count > 0
        && dot_count > 0
        && s.chars().next().is_some_and(|ch| ch.is_ascii_digit())
        && s.chars().last().is_some_and(|ch| ch.is_ascii_alphanumeric())
}

fn clean_asset_glob_prefix(s: &str) -> String {
    if s == "v" {
        return String::new();
    }
    for suffix in ["-v", "_v", ".v"] {
        if let Some(stripped) = s.strip_suffix(suffix) {
            let separator = suffix.chars().next().unwrap_or('-');
            return glob_escape(&format!("{stripped}{separator}"));
        }
    }
    glob_escape(s)
}

fn stable_asset_tokens(asset_name: &str, version_tag: &str) -> Vec<String> {
    let mut stripped = asset_name.to_string();
    for version in version_variants(version_tag) {
        stripped = stripped.replace(&version, " ");
    }

    let mut tokens: Vec<String> = stripped
        .split(['-', '_', ' ', '.'])
        .filter(|part| part.len() >= 3 && !part.chars().all(|ch| ch.is_ascii_digit()))
        .map(str::to_string)
        .collect();

    if let Some(ext) = asset_extension(asset_name) {
        tokens.push(ext);
    }

    tokens.sort();
    tokens.dedup();
    tokens
}

fn asset_extension(asset_name: &str) -> Option<String> {
    [".tar.gz", ".tgz", ".tar.xz", ".txz", ".tar.bz2", ".tbz", ".zip", ".7z", ".exe"]
        .iter()
        .find(|ext| asset_name.ends_with(**ext))
        .map(|ext| ext.trim_start_matches('.').to_string())
}

fn glob_escape(s: &str) -> String {
    s.chars()
        .flat_map(|ch| match ch {
            '*' | '?' | '\\' => ['\\', ch],
            _ => ['\0', ch],
        })
        .filter(|ch| *ch != '\0')
        .collect()
}

pub fn glob_match(pattern: &str, text: &str) -> bool {
    #[derive(Clone, Copy)]
    enum GlobToken {
        Star,
        Any,
        Literal(char),
    }

    let mut tokens = Vec::new();
    let mut chars = pattern.chars();
    while let Some(ch) = chars.next() {
        match ch {
            '*' => tokens.push(GlobToken::Star),
            '?' => tokens.push(GlobToken::Any),
            '\\' => tokens.push(GlobToken::Literal(chars.next().unwrap_or('\\'))),
            literal => tokens.push(GlobToken::Literal(literal)),
        }
    }

    let text: Vec<char> = text.chars().collect();
    let mut previous = vec![false; text.len() + 1];
    previous[0] = true;

    for token in tokens {
        let mut current = vec![false; text.len() + 1];
        match token {
            GlobToken::Star => {
                current[0] = previous[0];
                for idx in 1..=text.len() {
                    current[idx] = current[idx - 1] || previous[idx];
                }
            }
            GlobToken::Any => {
                current[1..=text.len()].copy_from_slice(&previous[..text.len()]);
            }
            GlobToken::Literal(ch) => {
                for idx in 1..=text.len() {
                    current[idx] = previous[idx - 1] && text[idx - 1] == ch;
                }
            }
        }
        previous = current;
    }

    previous[text.len()]
}

fn score_asset_selector(
    selector: &AssetSelector,
    asset_name: &str,
    version_tag: &str,
    all_asset_names: &[String],
) -> i32 {
    if !asset_matches_selector(asset_name, selector) {
        return i32::MIN;
    }

    let matches =
        all_asset_names.iter().filter(|name| asset_matches_selector(name, selector)).count();
    let mut score = 0;

    if matches == 1 {
        score += 1000;
    } else {
        score -= (matches as i32) * 100;
    }

    let selector_text = selector.to_string();
    let selector_lower = selector_text.to_lowercase();

    if matches!(selector, AssetSelector::Glob(_)) {
        score += 50;
    }

    for version in version_variants(version_tag) {
        if selector_lower.contains(&version.to_lowercase()) {
            score -= 1000;
        }
    }

    if selector_text.contains('*') {
        score += 500;
    }

    if [".tar.gz", ".tgz", ".tar.xz", ".txz", ".tar.bz2", ".tbz", ".zip", ".7z", ".exe"]
        .iter()
        .any(|ext| selector_lower.ends_with(ext))
    {
        score += 80;
    }

    if [
        "linux", "darwin", "apple", "macos", "windows", "win", "x86_64", "amd64", "x64", "aarch64",
        "arm64",
    ]
    .iter()
    .any(|part| selector_lower.contains(part))
    {
        score += 60;
    }

    score += selector_text.len().min(80) as i32;
    if selector_text.len() < 4 {
        score -= 200;
    }

    score
}

#[cfg(test)]
mod tests {
    use super::*;

    // ==================== is_ignored_format() ====================

    #[test]
    fn is_ignored_format_checksums() {
        assert!(is_ignored_format("file.sha256"));
        assert!(is_ignored_format("file.sha256sum"));
        assert!(is_ignored_format("file.md5"));
        assert!(is_ignored_format("file.asc"));
        assert!(is_ignored_format("file.sig"));
    }

    #[test]
    fn is_ignored_format_installers() {
        assert!(is_ignored_format("package.deb"));
        assert!(is_ignored_format("package.rpm"));
        assert!(is_ignored_format("package.msi"));
        assert!(is_ignored_format("package.pkg"));
        assert!(is_ignored_format("package.dmg"));
    }

    #[test]
    fn is_ignored_format_data_files() {
        assert!(is_ignored_format("readme.txt"));
        assert!(is_ignored_format("notes.md"));
        assert!(is_ignored_format("config.json"));
        assert!(is_ignored_format("data.xml"));
    }

    #[test]
    fn is_ignored_format_valid_archives_not_ignored() {
        assert!(!is_ignored_format("package.tar.gz"));
        assert!(!is_ignored_format("package.zip"));
        assert!(!is_ignored_format("package.7z"));
    }

    // ==================== is_supported_format() ====================

    #[test]
    fn is_supported_format_archives() {
        assert!(is_supported_format("package.tar.gz"));
        assert!(is_supported_format("package.tgz"));
        assert!(is_supported_format("package.tar.xz"));
        assert!(is_supported_format("package.txz"));
        assert!(is_supported_format("package.tar.bz2"));
        assert!(is_supported_format("package.tbz"));
        assert!(is_supported_format("package.7z"));
        assert!(is_supported_format("package.zip"));
    }

    #[test]
    fn is_supported_format_exe() {
        assert!(is_supported_format("binary.exe"));
    }

    #[test]
    fn is_supported_format_no_extension() {
        // ELF binaries often have no extension
        assert!(is_supported_format("ripgrep-linux-x86_64"));
    }

    #[test]
    fn is_supported_format_unknown_extension() {
        // Unknown extensions are not supported (need explicit config)
        assert!(!is_supported_format("file.unknown"));
        assert!(!is_supported_format("file.abc"));
    }

    // ==================== derive_asset_selector() ====================

    #[test]
    fn derive_selector_strips_v_prefixed_version() {
        assert_eq!(
            derive_asset_selector("ripgrep-15.1.0-x86_64-apple-darwin.tar.gz", "v15.1.0"),
            AssetSelector::Glob("ripgrep-*-x86_64-apple-darwin.tar.gz".to_string())
        );
    }

    #[test]
    fn derive_selector_strips_bare_version() {
        assert_eq!(
            derive_asset_selector("delta-0.18.2-x86_64-apple-darwin.tar.gz", "0.18.2"),
            AssetSelector::Glob("delta-*-x86_64-apple-darwin.tar.gz".to_string())
        );
    }

    #[test]
    fn derive_selector_with_underscore_separator() {
        assert_eq!(
            derive_asset_selector("fd_10.2.0_amd64.deb", "v10.2.0"),
            AssetSelector::Glob("fd_*_amd64.deb".to_string())
        );
    }

    #[test]
    fn derive_selector_fallback_when_version_not_in_name() {
        assert_eq!(
            derive_asset_selector("tool-linux-amd64", "v1.0.0"),
            AssetSelector::Glob("tool-linux-amd64".to_string())
        );
    }

    #[test]
    fn derive_selector_version_at_end_uses_prefix() {
        assert_eq!(
            derive_asset_selector("tool-1.0.0", "v1.0.0"),
            AssetSelector::Glob("tool-*".to_string())
        );
    }

    #[test]
    fn derive_selector_v_prefix_in_name_uses_prefix() {
        assert_eq!(
            derive_asset_selector("tool-v1.0.0", "v1.0.0"),
            AssetSelector::Glob("tool-*".to_string())
        );
    }

    #[test]
    fn derive_selector_version_at_end_bare_tag_uses_prefix() {
        assert_eq!(
            derive_asset_selector("tool-1.0.0", "1.0.0"),
            AssetSelector::Glob("tool-*".to_string())
        );
    }

    #[test]
    fn derive_selector_version_at_start_uses_suffix() {
        assert_eq!(
            derive_asset_selector("v1.0.0-tool-linux-x86_64.tar.gz", "v1.0.0"),
            AssetSelector::Glob("*-tool-linux-x86_64.tar.gz".to_string())
        );
    }

    #[test]
    fn derive_selector_prefers_unique_versionless_glob() {
        let assets = vec![
            "tool-v1.0.0-linux-x86_64.tar.gz".to_string(),
            "tool-v1.0.0-linux-aarch64.tar.gz".to_string(),
            "tool-v1.0.0-windows-x86_64.zip".to_string(),
        ];

        assert_eq!(
            derive_asset_selector_from_assets("tool-v1.0.0-linux-x86_64.tar.gz", "v1.0.0", &assets),
            AssetSelector::Glob("tool-*-linux-x86_64.tar.gz".to_string())
        );
    }

    #[test]
    fn derive_selector_handles_asset_with_v_when_tag_is_bare() {
        assert_eq!(
            derive_asset_selector("tool-v1.0.0-linux.tar.gz", "1.0.0"),
            AssetSelector::Glob("tool-*-linux.tar.gz".to_string())
        );
    }

    #[test]
    fn derive_selector_handles_release_prefixed_tag() {
        assert_eq!(
            derive_asset_selector("aria2-1.37.0.tar.xz", "release-1.37.0"),
            AssetSelector::Glob("aria2-*.tar.xz".to_string())
        );
    }

    #[test]
    fn version_variants_do_not_prefix_non_plain_versions() {
        let variants = version_variants("release-1.37.0");

        assert!(variants.contains(&"release-1.37.0".to_string()));
        assert!(variants.contains(&"1.37.0".to_string()));
        assert!(!variants.contains(&"vrelease-1.37.0".to_string()));
    }

    #[test]
    fn version_variants_prefix_plain_versions() {
        let variants = version_variants("1.37.0");

        assert!(variants.contains(&"1.37.0".to_string()));
        assert!(variants.contains(&"v1.37.0".to_string()));
    }

    #[test]
    fn asset_selector_tokens_match_all_parts() {
        let selector = AssetSelector::Tokens(vec!["tool".to_string(), "linux.tar.gz".to_string()]);
        assert!(asset_matches_selector("tool-v1.0.0-linux.tar.gz", &selector));
        assert!(!asset_matches_selector("other-v1.0.0-linux.tar.gz", &selector));
    }

    #[test]
    fn glob_match_supports_star_question_and_escape() {
        assert!(glob_match("aria2-*.tar.xz", "aria2-1.37.0.tar.xz"));
        assert!(glob_match("tool-?.zip", "tool-a.zip"));
        assert!(!glob_match("tool-?.zip", "tool-ab.zip"));
        assert!(glob_match(r"literal-\*.zip", "literal-*.zip"));
    }

    #[test]
    fn glob_match_handles_many_stars_iteratively() {
        let pattern = "*a*a*a*a*a*a*a*a*a*a*z";
        let text = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaz";

        assert!(glob_match(pattern, text));
        assert!(!glob_match(pattern, "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"));
    }
}
