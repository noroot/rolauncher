// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2025-2026 dmth

//! Discovers installed `.app` bundles and searches them by name.

use std::path::{Path, PathBuf};

/// Directories scanned for applications (non-recursive), same as the Swift version.
pub const APP_DIRECTORIES: &[&str] = &[
    "/Applications",
    "/System/Applications",
    "/System/Applications/Utilities",
    "/System/Library/CoreServices",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppEntry {
    pub name: String,
    pub path: PathBuf,
    lower: String,
}

impl AppEntry {
    pub fn new(name: impl Into<String>, path: impl Into<PathBuf>) -> Self {
        let name = name.into();
        let lower = name.to_lowercase();
        Self { name, path: path.into(), lower }
    }
}

/// Scans `dirs` for `.app` bundles. `display_name` resolves a bundle's
/// user-facing name (e.g. `CFBundleDisplayName`), falling back to the file stem.
pub fn scan<F>(dirs: &[&str], display_name: F) -> Vec<AppEntry>
where
    F: Fn(&Path) -> Option<String>,
{
    let mut apps: Vec<AppEntry> = dirs
        .iter()
        .filter_map(|dir| std::fs::read_dir(dir).ok())
        .flatten()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "app"))
        // Skip hidden bundles such as `.Karabiner-VirtualHIDDevice-Manager.app`.
        .filter(|path| !path.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.')))
        .map(|path| {
            let name = display_name(&path)
                .filter(|n| !n.trim().is_empty())
                .unwrap_or_else(|| path.file_stem().unwrap_or_default().to_string_lossy().into_owned());
            AppEntry::new(name, path)
        })
        .collect();
    apps.sort_by(|a, b| a.lower.cmp(&b.lower).then_with(|| a.path.cmp(&b.path)));
    apps
}

/// Case-insensitive substring match. Ranking: names starting with the query first,
/// then by `usage` (higher first), then alphabetical. An empty query returns every app,
/// so it is ordered purely by usage.
pub fn search<'a, K: Ord>(apps: &'a [AppEntry], query: &str, usage: impl Fn(&AppEntry) -> K) -> Vec<&'a AppEntry> {
    let q = query.trim().to_lowercase();
    let mut hits: Vec<&AppEntry> = apps.iter().filter(|a| a.lower.contains(&q)).collect();
    // `apps` is already alphabetical, so a stable sort keeps that as the final tiebreak.
    hits.sort_by_cached_key(|a| (!a.lower.starts_with(&q), std::cmp::Reverse(usage(a))));
    hits
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(v: Vec<&AppEntry>) -> Vec<&str> {
        v.into_iter().map(|a| a.name.as_str()).collect()
    }

    #[test]
    fn search_ranks_prefix_matches_first() {
        let apps = vec![
            AppEntry::new("Calculator", "/a"),
            AppEntry::new("Calendar", "/b"),
            AppEntry::new("iCal Sync", "/c"),
            AppEntry::new("Safari", "/d"),
        ];
        let none = |_: &AppEntry| 0;
        assert_eq!(names(search(&apps, "cal", none)), ["Calculator", "Calendar", "iCal Sync"]);
        assert_eq!(names(search(&apps, "SAF", none)), ["Safari"]);
        assert_eq!(search(&apps, "", none).len(), 4);
        assert!(search(&apps, "zzz", none).is_empty());
    }

    #[test]
    fn search_orders_by_usage_within_prefix_groups() {
        let apps = vec![
            AppEntry::new("Calculator", "/a"),
            AppEntry::new("Calendar", "/b"),
            AppEntry::new("iCal Sync", "/c"),
            AppEntry::new("Safari", "/d"),
        ];
        let usage = |a: &AppEntry| match a.name.as_str() {
            "Calendar" => 5,
            "iCal Sync" => 9,
            "Safari" => 7,
            _ => 0,
        };
        // Empty query: pure usage order, unused apps alphabetical at the end.
        assert_eq!(names(search(&apps, "", usage)), ["iCal Sync", "Safari", "Calendar", "Calculator"]);
        // Prefix matches still beat a more-used substring match.
        assert_eq!(names(search(&apps, "cal", usage)), ["Calendar", "Calculator", "iCal Sync"]);
    }

    #[test]
    fn scan_finds_apps_and_uses_display_name() {
        let dir = std::env::temp_dir().join(format!("rl-scan-{}", std::process::id()));
        for d in ["Zed.app", "alpha.app", "notes.txt", "Beta.app", ".Hidden.app"] {
            std::fs::create_dir_all(dir.join(d)).unwrap();
        }
        let dir_str = dir.to_str().unwrap();
        let apps = scan(&[dir_str, "/nonexistent"], |p| {
            (p.file_stem()? == "Beta").then(|| "Beta Pro".to_string())
        });
        std::fs::remove_dir_all(&dir).unwrap();
        let got: Vec<_> = apps.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(got, ["alpha", "Beta Pro", "Zed"]);
    }
}
