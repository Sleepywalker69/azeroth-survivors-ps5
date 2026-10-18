//! The player's own files: the AddOns folder the client reads its addons from, and the save
//! files the Options screen exports and imports. In a browser these live in IndexedDB and pass
//! through file pickers; here they are folders in the player's folder.

use std::path::{Path, PathBuf};

use serde_json::json;

use crate::log::say;

pub struct Files {
    /// The AddOns folder.
    pub addons: PathBuf,
    /// Exported saves go here; a save to import is put in its `import` folder.
    pub saves: PathBuf,
}

impl Files {
    pub fn new(home: &Path) -> Files {
        let files = Files { addons: home.join("AddOns"), saves: home.join("Saves") };
        for dir in [&files.addons, &files.saves, &files.saves.join("import")] {
            let _ = std::fs::create_dir_all(dir);
        }
        files
    }

    /// `rel` (lowercased, '/'-separated, '' for the folder itself) found under the AddOns folder
    /// without regard to case.
    fn resolve(&self, rel: &str) -> Option<PathBuf> {
        let mut path = self.addons.clone();
        for part in rel.split('/').filter(|p| !p.is_empty()) {
            if part == "." || part == ".." {
                return None;
            }
            let exact = path.join(part);
            if exact.exists() {
                path = exact;
                continue;
            }
            let entry = std::fs::read_dir(&path)
                .ok()?
                .flatten()
                .find(|e| e.file_name().to_string_lossy().to_lowercase() == part)?;
            path = entry.path();
        }
        Some(path)
    }

    pub fn read(&self, rel: &str) -> Option<Vec<u8>> {
        let path = self.resolve(rel)?;
        path.is_file().then(|| std::fs::read(&path).ok()).flatten()
    }

    /// The names in a folder as they are written, as a JSON array.
    pub fn list(&self, rel: &str) -> String {
        let names: Vec<String> = self
            .resolve(rel)
            .and_then(|p| std::fs::read_dir(p).ok())
            .map(|entries| entries.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect())
            .unwrap_or_default();
        json!(names).to_string()
    }

    /// 'file', 'dir' or '' for nothing there.
    pub fn kind(&self, rel: &str) -> &'static str {
        match self.resolve(rel) {
            Some(p) if p.is_dir() => "dir",
            Some(p) if p.is_file() => "file",
            _ => "",
        }
    }

    /// Deletes an installed addon's folder.
    pub fn remove_addon(&self, name: &str) -> Result<(), String> {
        if name.is_empty() || name.contains(['/', '\\']) || name == "." || name == ".." {
            return Err(format!("{name:?} is not an addon's name"));
        }
        let path = self.resolve(&name.to_lowercase()).ok_or_else(|| format!("{name} is not installed"))?;
        std::fs::remove_dir_all(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        say!("files: removed the addon {}", path.display());
        Ok(())
    }

    /// Writes an exported save to a new file; its path.
    pub fn export_save(&self, text: &str) -> Result<String, String> {
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let path = self.saves.join(format!("AzerothSurvivors {secs}.sav"));
        std::fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(path.display().to_string())
    }

    /// The newest `.sav` or `.txt` file in the import folder, as `{ name, text }`.
    pub fn pick_save(&self) -> Option<String> {
        let dir = self.saves.join("import");
        let newest = std::fs::read_dir(&dir)
            .ok()?
            .flatten()
            .filter(|e| {
                let name = e.file_name().to_string_lossy().to_lowercase();
                name.ends_with(".sav") || name.ends_with(".txt")
            })
            .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
            .max_by_key(|(time, _)| *time)?;
        let text = std::fs::read_to_string(&newest.1).ok()?;
        let name = newest.1.file_name()?.to_string_lossy().into_owned();
        say!("files: picked the save {}", newest.1.display());
        Some(json!({ "name": name, "text": text }).to_string())
    }
}
