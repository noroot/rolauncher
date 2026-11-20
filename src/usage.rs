// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2025-2026 dmth

//! Launch-count tracking, persisted as a small TSV file:
//! `~/Library/Application Support/RoLauncher/usage.tsv` with lines `count<TAB>last_unix_secs<TAB>path`.

use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Stat {
    pub count: u32,
    pub last_used: u64,
}

#[derive(Debug, Default)]
pub struct Usage {
    file: Option<PathBuf>,
    stats: HashMap<PathBuf, Stat>,
}

impl Usage {
    /// Default on-disk location; `None` if `$HOME` is unset.
    pub fn default_file() -> Option<PathBuf> {
        let home = std::env::var_os("HOME")?;
        Some(PathBuf::from(home).join("Library/Application Support/RoLauncher/usage.tsv"))
    }

    /// Loads stats from `file`; a missing or malformed file yields empty/partial stats.
    pub fn load(file: Option<PathBuf>) -> Self {
        let stats = file
            .as_deref()
            .and_then(|f| std::fs::read_to_string(f).ok())
            .map(|text| parse(&text))
            .unwrap_or_default();
        Self { file, stats }
    }

    pub fn get(&self, path: &Path) -> Stat {
        self.stats.get(path).copied().unwrap_or_default()
    }

    /// Records one launch and persists immediately.
    pub fn record(&mut self, path: &Path) -> io::Result<()> {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
        let stat = self.stats.entry(path.to_path_buf()).or_default();
        stat.count = stat.count.saturating_add(1);
        stat.last_used = now;
        self.save()
    }

    fn save(&self) -> io::Result<()> {
        let Some(file) = &self.file else { return Ok(()) };
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let mut out = String::new();
        for (path, s) in &self.stats {
            out.push_str(&format!("{}\t{}\t{}\n", s.count, s.last_used, path.display()));
        }
        // Write-then-rename so a crash never leaves a truncated file.
        let tmp = file.with_extension("tsv.tmp");
        std::fs::write(&tmp, out)?;
        std::fs::rename(tmp, file)
    }
}

fn parse(text: &str) -> HashMap<PathBuf, Stat> {
    text.lines()
        .filter_map(|line| {
            let mut parts = line.splitn(3, '\t');
            let count = parts.next()?.parse().ok()?;
            let last_used = parts.next()?.parse().ok()?;
            let path = parts.next().filter(|p| !p.is_empty())?;
            Some((PathBuf::from(path), Stat { count, last_used }))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_persists_and_reloads() {
        let dir = std::env::temp_dir().join(format!("rl-usage-{}", std::process::id()));
        let file = dir.join("usage.tsv");
        let mut usage = Usage::load(Some(file.clone()));
        usage.record(Path::new("/Applications/Safari.app")).unwrap();
        usage.record(Path::new("/Applications/Safari.app")).unwrap();
        usage.record(Path::new("/Applications/My App.app")).unwrap();

        let reloaded = Usage::load(Some(file));
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(reloaded.get(Path::new("/Applications/Safari.app")).count, 2);
        assert_eq!(reloaded.get(Path::new("/Applications/My App.app")).count, 1);
        assert_eq!(reloaded.get(Path::new("/Applications/Other.app")), Stat::default());
    }

    #[test]
    fn parse_skips_malformed_lines() {
        let stats = parse("3\t100\t/A.app\ngarbage\n\t\t\n1\t5\t/B.app\n");
        assert_eq!(stats.len(), 2);
        assert_eq!(stats[Path::new("/A.app")], Stat { count: 3, last_used: 100 });
    }
}
