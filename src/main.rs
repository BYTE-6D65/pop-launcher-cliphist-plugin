// Cliphist plugin for pop-launcher (COSMIC launcher).
//
// KNOWN QUIRK: pop-launcher sends different queries for `!` vs `! ` (bang alone
// vs bang+space). The no-query sort order differs from the with-query sort order
// and we never figured out why -- pop-launcher appears to apply its own secondary
// ranking that we can't control. The with-query (`! `) path sorts correctly by
// recency. The no-query (`!`) path is "close enough." This is stupid and we'll
// revisit it when someone cares enough to debug pop-launcher's internal sort.
//   -- Byte & Annie, May 2026

use fuzzy_matcher::FuzzyMatcher;
use pop_launcher_toolkit::launcher::{Indice, PluginResponse, PluginSearchResult};
use pop_launcher_toolkit::plugin_trait::{async_trait, PluginExt};
use std::process::Stdio;
use tokio::process::Command;

struct Entry {
    cliphist_id: String,
    content: String,
}

struct CliphistPlugin {
    entries: Vec<Entry>,
}

#[async_trait]
impl PluginExt for CliphistPlugin {
    fn name(&self) -> &str {
        "cliphist"
    }

    async fn search(&mut self, query: &str) {
        // Refresh entries from cliphist on every search (no stale cache)
        self.entries = load_entries().await;
        let total = self.entries.len();

        // Strip the ! prefix that pop-launcher passes through
        let query = query.strip_prefix('!').unwrap_or(query).trim();

        let results: Vec<(u64, usize)> = if query.is_empty() {
            // No query: return all, scored by recency (exponential decay).
            // See top-of-file comment about why this doesn't always sort right.
            self.entries
                .iter()
                .enumerate()
                .map(|(i, _)| {
                    let score = if total > i { ((total - i) as u64).pow(2) } else { 0 };
                    (score, i)
                })
                .collect()
        } else {
            // Fuzzy search: blend match quality with recency
            let matcher = fuzzy_matcher::skim::SkimMatcherV2::default();
            self.entries
                .iter()
                .enumerate()
                .filter_map(|(i, e)| {
                    let fuzzy = matcher.fuzzy_match(&e.content, query)? as u64;
                    let recency = ((total - i) as f64 / total as f64 * 100.0) as u64;
                    let blended = fuzzy * 8 / 10 + recency * 2 / 10;
                    Some((blended, i))
                })
                .collect()
        };

        let mut sorted = results;
        sorted.sort_by(|a, b| b.0.cmp(&a.0));

        for (_, idx) in sorted.iter().take(20) {
            let entry = &self.entries[*idx];
            self.respond_with(PluginResponse::Append(PluginSearchResult {
                id: *idx as Indice,
                name: entry.content.clone(),
                description: String::new(),
                keywords: None,
                icon: None,
                exec: None,
                window: None,
            }))
            .await;
        }

        self.respond_with(PluginResponse::Finished).await;
    }

    async fn activate(&mut self, id: Indice) {
        if let Some(entry) = self.entries.get(id as usize) {
            // Decode cliphist entry back to both clipboards
            let _ = Command::new("sh")
                .arg("-c")
                .arg(format!("cliphist decode {} | wl-copy", entry.cliphist_id))
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .await;

            let _ = Command::new("sh")
                .arg("-c")
                .arg(format!(
                    "cliphist decode {} | wl-copy --primary",
                    entry.cliphist_id
                ))
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .await;
        }

        self.respond_with(PluginResponse::Close).await;
    }
}

async fn load_entries() -> Vec<Entry> {
    let output = Command::new("cliphist")
        .arg("list")
        .output()
        .await
        .ok();

    let stdout = match output {
        Some(o) => o.stdout,
        None => return Vec::new(),
    };

    let text = String::from_utf8_lossy(&stdout);
    let mut entries = Vec::new();

    for line in text.lines() {
        if let Some((id, content)) = line.split_once('\t') {
            // First line of content, truncated to 120 chars
            let clean = content
                .lines()
                .next()
                .unwrap_or("")
                .chars()
                .take(120)
                .collect::<String>();

            entries.push(Entry {
                cliphist_id: id.to_string(),
                content: clean,
            });
        }
    }

    entries
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let mut plugin = CliphistPlugin { entries: Vec::new() };
    plugin.run().await;
}
