use crate::config::{Config, atomic_write};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Entry {
    pub path: PathBuf,
    pub created: String,
}
#[derive(Default, Serialize, Deserialize)]
pub struct History {
    pub entries: Vec<Entry>,
}
impl History {
    pub fn path() -> PathBuf {
        Config::path().with_file_name("history.toml")
    }
    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|s| toml::from_str(&s).ok())
            .unwrap_or_default()
    }
    pub fn record(&mut self, entry: Entry, limit: usize) {
        self.entries.retain(|e| e.path != entry.path);
        self.entries.insert(0, entry);
        self.entries.truncate(limit);
    }
    pub fn save(&self, path: &Path) -> Result<()> {
        atomic_write(path, toml::to_string_pretty(self)?.as_bytes())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn retention_keeps_exported_files() {
        let dir = tempfile::tempdir().unwrap();
        let mut history = History::default();
        for i in 0..5 {
            let path = dir.path().join(format!("{i}.png"));
            std::fs::write(&path, b"fixture").unwrap();
            history.record(
                Entry {
                    path,
                    created: i.to_string(),
                },
                2,
            );
        }
        assert_eq!(history.entries.len(), 2);
        assert!(dir.path().join("0.png").exists());
        let path = dir.path().join("history.toml");
        history.save(&path).unwrap();
        assert_eq!(History::load(&path).entries.len(), 2);
    }
}
