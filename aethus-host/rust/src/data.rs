//! The game data: the page's `/data/<path>` requests, answered from the MPQ chain the web build's
//! server serves its files from.
//!
//! The chain is mounted the way the client mounts an install's `Data` folder (benilla-formats'
//! `chain.rs`): the base archives in their fixed order, then `patch.MPQ`, then `patch-?.MPQ` in
//! name order, then `speech2.MPQ`; a later archive wins, and a delete marker removes a name.

use std::path::PathBuf;

use benilla_mpq::Archive;

use crate::log::{say, warn};

const BASE_ORDER: [&str; 10] = [
    "dbc.mpq",
    "speech.mpq",
    "fonts.mpq",
    "interface.mpq",
    "misc.mpq",
    "sound.mpq",
    "wmo.mpq",
    "terrain.mpq",
    "texture.mpq",
    "model.mpq",
];

pub struct Data {
    /// Oldest first.
    chain: Vec<Archive>,
}

/// Where an archive goes in the chain, or None for a name the chain does not mount.
fn rank(name: &str) -> Option<(u32, String)> {
    let lower = name.to_ascii_lowercase();
    if let Some(i) = BASE_ORDER.iter().position(|b| *b == lower) {
        return Some((i as u32, lower));
    }
    if lower == "patch.mpq" {
        return Some((100, lower));
    }
    let middle = lower.strip_prefix("patch-").and_then(|s| s.strip_suffix(".mpq"));
    if middle.is_some_and(|m| m.chars().count() == 1) {
        return Some((200, lower));
    }
    if lower == "speech2.mpq" {
        return Some((300, lower));
    }
    None
}

impl Data {
    /// Mounts the archives found in `dirs`. When two folders hold the same archive name, the
    /// later folder's copy is used.
    pub fn mount(dirs: &[PathBuf]) -> Data {
        let mut found: Vec<((u32, String), PathBuf)> = Vec::new();
        for dir in dirs {
            let entries = match std::fs::read_dir(dir) {
                Ok(entries) => entries,
                Err(e) => {
                    warn!("data: {} cannot be read: {e}", dir.display());
                    continue;
                }
            };
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                let Some(rank) = rank(&name) else { continue };
                found.retain(|(r, _)| r.1 != rank.1);
                found.push((rank, entry.path()));
            }
        }
        found.sort_by(|a, b| a.0.cmp(&b.0));
        let mut chain = Vec::new();
        for (_, path) in found {
            match Archive::open(&path) {
                Ok(archive) => {
                    say!("data: mounted {}", path.display());
                    chain.push(archive);
                }
                Err(e) => warn!("data: {} was not mounted: {e}", path.display()),
            }
        }
        if chain.is_empty() {
            warn!("data: no archives are mounted; the client will find none of its files");
        }
        Data { chain }
    }

    /// The archive that answers for `name`, or None when no archive has it or the newest one
    /// that names it deletes it.
    fn find(&self, name: &str) -> Option<&Archive> {
        for archive in self.chain.iter().rev() {
            if archive.contains(name) {
                return (!archive.is_delete_marker(name)).then_some(archive);
            }
        }
        None
    }

    pub fn exists(&self, name: &str) -> bool {
        self.find(name).is_some()
    }

    pub fn read(&self, name: &str) -> Option<Vec<u8>> {
        let archive = self.find(name)?;
        match archive.read_file(name) {
            Ok(bytes) => Some(bytes),
            Err(e) => {
                warn!("data: {name} in {}: {e}", archive.path().display());
                None
            }
        }
    }

    pub fn archive_count(&self) -> usize {
        self.chain.len()
    }
}
