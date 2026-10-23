use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Write,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Debug, Serialize, Deserialize)]
pub struct Entry {
    pub timestamp: u64,
    pub kind: String,
    pub text: String,
}
impl Entry {
    pub fn date(&self) -> String {
        let timestamp = self.timestamp as libc::time_t;
        let mut local = std::mem::MaybeUninit::<libc::tm>::uninit();
        // localtime_r writes into our own buffer and does not use shared state.
        unsafe {
            if libc::localtime_r(&timestamp, local.as_mut_ptr()).is_null() {
                return self.timestamp.to_string();
            }
            let t = local.assume_init();
            format!(
                "{:02}/{:02}/{:04} {:02}:{:02}:{:02}",
                t.tm_mday,
                t.tm_mon + 1,
                t.tm_year + 1900,
                t.tm_hour,
                t.tm_min,
                t.tm_sec
            )
        }
    }
}
pub struct Journal {
    pub entries: Vec<Entry>,
    path: PathBuf,
    days: u16,
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
impl Journal {
    pub fn open(path: PathBuf, days: u16) -> Result<Self> {
        let entries = match fs::read_to_string(&path) {
            Ok(s) => s
                .lines()
                .filter(|s| !s.trim().is_empty())
                .map(serde_json::from_str)
                .collect::<std::result::Result<Vec<_>, _>>()
                .context("Registro attività non valido")?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => vec![],
            Err(e) => return Err(e).context("Lettura registro attività"),
        };
        let mut journal = Self {
            entries,
            path,
            days,
        };
        journal.prune()?;
        Ok(journal)
    }
    fn persist(&self) -> Result<()> {
        let parent = self.path.parent().context("Directory registro assente")?;
        fs::create_dir_all(parent)?;
        let mut file = tempfile::NamedTempFile::new_in(parent)?;
        for entry in &self.entries {
            serde_json::to_writer(&mut file, entry)?;
            writeln!(file)?;
        }
        file.as_file().sync_all()?;
        file.persist(&self.path)?;
        Ok(())
    }
    fn prune(&mut self) -> Result<()> {
        if self.days == 0 {
            return Ok(());
        }
        let cutoff = now().saturating_sub(u64::from(self.days) * 86400);
        if self.entries.iter().any(|e| e.timestamp < cutoff) {
            let previous = std::mem::take(&mut self.entries);
            self.entries = previous
                .iter()
                .filter(|e| e.timestamp >= cutoff)
                .map(|e| Entry {
                    timestamp: e.timestamp,
                    kind: e.kind.clone(),
                    text: e.text.clone(),
                })
                .collect();
            if let Err(e) = self.persist() {
                self.entries = previous;
                return Err(e);
            }
        }
        Ok(())
    }
    pub fn append(&mut self, kind: &str, text: String) -> Result<()> {
        self.prune()?;
        let entry = Entry {
            timestamp: now(),
            kind: kind.into(),
            text,
        };
        fs::create_dir_all(self.path.parent().context("Directory registro assente")?)?;
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        let mut data = serde_json::to_vec(&entry)?;
        data.push(b'\n');
        file.write_all(&data)?;
        file.sync_data()?;
        self.entries.push(entry);
        Ok(())
    }
    pub fn set_retention(&mut self, days: u16) -> Result<()> {
        let previous = self.days;
        self.days = days;
        if let Err(e) = self.prune() {
            self.days = previous;
            return Err(e);
        }
        Ok(())
    }
    pub fn clear(&mut self) -> Result<()> {
        let previous = std::mem::take(&mut self.entries);
        if let Err(e) = self.persist() {
            self.entries = previous;
            return Err(e);
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn persists_prunes_and_clears() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("activity.jsonl");
        let mut log = Journal::open(path.clone(), 0)?;
        log.append("Errore", "Rete\nnon disponibile".into())?;
        log.entries.push(Entry {
            timestamp: now() - 10 * 86400,
            kind: "App".into(),
            text: "Vecchio".into(),
        });
        log.persist()?;
        let mut log = Journal::open(path.clone(), 30)?;
        assert_eq!(log.entries.len(), 2);
        log.set_retention(7)?;
        assert_eq!(Journal::open(path.clone(), 7)?.entries.len(), 1);
        assert_eq!(log.entries[0].text, "Rete\nnon disponibile");
        log.clear()?;
        assert!(Journal::open(path, 30)?.entries.is_empty());
        Ok(())
    }
}
