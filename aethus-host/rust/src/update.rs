//! Console update transactions. No game state or network API is used here: the same code is
//! exercised on the build machine, including interruption and rollback tests.
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(target_env = "ps5")]
pub mod net_ps5;
#[cfg(target_env = "ps5")]
pub mod runtime_ps5;

pub type Result<T> = std::result::Result<T, String>;
pub const MANIFEST_LIMIT: u64 = 1024 * 1024;
const TOTAL_LIMIT: u64 = 8 * 1024 * 1024 * 1024;

fn io<T>(r: std::io::Result<T>) -> Result<T> { r.map_err(|e| e.to_string()) }
fn text<'a>(v: &'a Value, k: &str) -> Result<&'a str> {
    v[k].as_str().ok_or_else(|| format!("missing string: {k}"))
}
fn number(v: &Value, k: &str) -> Result<u64> {
    v[k].as_u64().ok_or_else(|| format!("missing integer: {k}"))
}
fn identifier(s: &str) -> bool {
    !s.is_empty() && s.len() <= 96 && s.bytes().all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c))
        && s != "." && s != ".." && !s.ends_with('.')
}
fn digest_text(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}
fn hex(bytes: &[u8]) -> String { bytes.iter().map(|b| format!("{b:02x}")).collect() }
pub fn hash_bytes(bytes: &[u8]) -> String { hex(&Sha256::digest(bytes)) }
pub fn hash_file(path: &Path) -> Result<(u64, String)> {
    let mut file = io(File::open(path))?;
    let mut buffer = [0; 256 * 1024];
    let mut hash = Sha256::new();
    let mut size = 0;
    loop {
        let n = io(file.read(&mut buffer))?;
        if n == 0 { break; }
        size += n as u64;
        hash.update(&buffer[..n]);
    }
    Ok((size, hex(&hash.finalize())))
}

/// Reject aliases before joining. Update payloads cannot reach saves, configuration or base MPQs.
fn payload_path(path: &str) -> bool {
    if path.len() > 240 || path.split('/').any(|p| !identifier(p) || p.ends_with('.')) { return false; }
    path == "eboot.bin" || path == "sce_module/libc.prx"
        || matches!(path, "sce_sys/param.json" | "sce_sys/icon0.png" | "sce_sys/pic0.dds" | "sce_sys/pic1.dds" | "sce_sys/snd0.at9")
        || path.starts_with("js/") || path.starts_with("web/") || path.starts_with("licenses/")
        || path.strip_prefix("Data/").is_some_and(|p| {
            p == "patch.MPQ" || p.strip_prefix("patch-").and_then(|p| p.strip_suffix(".MPQ"))
                .is_some_and(|n| !n.is_empty() && n.len() < 6 && n.bytes().all(|b| b.is_ascii_digit()))
        })
}
fn checked_join(root: &Path, relative: &str) -> Result<PathBuf> {
    if relative.split('/').any(|c| !identifier(c)) { return Err("unsafe relative path".into()); }
    let mut path = root.to_owned();
    for part in relative.split('/') {
        path.push(part);
        match fs::symlink_metadata(&path) {
            Ok(m) if m.file_type().is_symlink() => return Err(format!("symlink in update path: {}", path.display())),
            Ok(_) => (),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            Err(e) => return Err(e.to_string()),
        }
    }
    Ok(path)
}

#[derive(Clone)]
pub struct Policy { pub manifest_url: String, pub manifest_sha256: Option<String>, authority: String, local: bool }
fn url_parts(url: &str) -> Result<(&str, &str)> {
    if url.len() > 8192 || url.bytes().any(|b| b <= 32 || b >= 127 || b"\\@#".contains(&b)) {
        return Err("invalid update URL".into());
    }
    let (scheme, tail) = url.split_once("://").ok_or("update URL has no scheme")?;
    let authority = tail.split('/').next().unwrap_or("");
    if authority.is_empty() || authority.contains('?') { return Err("invalid update authority".into()); }
    Ok((scheme, authority))
}
impl Policy {
    pub fn new(url: &str, pin: Option<&str>) -> Result<Self> {
        let (scheme, authority) = url_parts(url)?;
        if pin.is_some_and(|s| !digest_text(s)) { return Err("invalid manifest SHA-256".into()); }
        let address = authority.split(':').next().unwrap_or("").parse::<std::net::Ipv4Addr>();
        let local = scheme == "http" && pin.is_some() && address.is_ok_and(|ip| ip.is_private() || ip.is_loopback());
        if scheme != "https" && !local { return Err("updates require HTTPS; a pinned private-LAN test feed is also supported".into()); }
        Ok(Self { manifest_url: url.into(), manifest_sha256: pin.map(str::to_owned), authority: authority.into(), local })
    }
    pub fn check_url(&self, url: &str, redirect: bool) -> Result<()> {
        let (scheme, authority) = url_parts(url)?;
        if self.local {
            if scheme == "http" && authority == self.authority { return Ok(()); }
        } else if scheme == "https" && (authority == self.authority
            || redirect && self.authority == "github.com" && authority == "release-assets.githubusercontent.com") {
            return Ok(());
        }
        Err("update URL left the configured feed or downgraded transport".into())
    }
}

#[derive(Clone, Debug)]
pub struct Asset { pub path: String, pub url: String, pub size: u64, pub sha256: String }
#[derive(Clone, Debug)]
pub struct Manifest { pub id: String, pub version: String, pub sequence: u64, pub files: Vec<Asset>, pub raw: Value }
impl Manifest {
    pub fn parse(bytes: &[u8], policy: &Policy) -> Result<Self> {
        if bytes.len() as u64 > MANIFEST_LIMIT { return Err("oversized manifest".into()); }
        if policy.manifest_sha256.as_ref().is_some_and(|s| *s != hash_bytes(bytes)) { return Err("manifest SHA-256 mismatch".into()); }
        let raw: Value = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        Self::from_value(raw, Some(policy))
    }
    fn from_value(raw: Value, policy: Option<&Policy>) -> Result<Self> {
        if number(&raw, "format")? != 1 || text(&raw, "title_id")? != "PPSA99320" {
            return Err("unsupported update format or title".into());
        }
        let id = text(&raw, "id")?.to_owned();
        let version = text(&raw, "version")?.to_owned();
        if !identifier(&id) || !identifier(&version) { return Err("invalid release identity".into()); }
        let sequence = number(&raw, "sequence")?;
        if sequence == 0 { return Err("release sequence must be positive".into()); }
        let entries = raw["files"].as_array().ok_or("missing file list")?;
        if entries.is_empty() || entries.len() > 1024 { return Err("invalid file count".into()); }
        let mut names = HashSet::new();
        let mut total: u64 = 0;
        let mut files = Vec::new();
        for f in entries {
            let path = text(f, "path")?;
            let size = number(f, "size")?;
            let sha256 = text(f, "sha256")?;
            let url = text(f, "url")?;
            let limit = if path.starts_with("Data/") { 4 * 1024 * 1024 * 1024 }
                else if path == "eboot.bin" || path == "sce_module/libc.prx" { 512 * 1024 * 1024 }
                else { 16 * 1024 * 1024 };
            if !payload_path(path) || !names.insert(path.to_ascii_lowercase()) || size > limit || !digest_text(sha256) {
                return Err(format!("invalid or duplicate update file: {path}"));
            }
            if let Some(p) = policy { p.check_url(url, false)?; }
            total = total.checked_add(size).ok_or("size overflow")?;
            if total > TOTAL_LIMIT { return Err("update exceeds total size limit".into()); }
            files.push(Asset { path: path.into(), url: url.into(), size, sha256: sha256.into() });
        }
        for required in ["eboot.bin", "js/boot.mjs", "web/wenilla.js", "sce_sys/param.json"] {
            if !files.iter().any(|f| f.path == required) { return Err(format!("incomplete release: {required}")); }
        }
        for name in &names {
            let mut parent = name.as_str();
            while let Some((p, _)) = parent.rsplit_once('/') {
                if names.contains(p) { return Err("file/directory collision".into()); }
                parent = p;
            }
        }
        files.sort_by_key(|f| (f.path == "eboot.bin", f.path.clone())); // executable is committed last
        Ok(Self { id, version, sequence, files, raw })
    }
}

pub trait Fetch {
    /// Must reject non-200 responses and stop after `limit` bytes, including unknown lengths.
    fn get(&mut self, url: &str, policy: &Policy, out: &mut dyn Write, limit: u64) -> Result<()>;
}

fn matches(path: &Path, asset: &Asset) -> bool {
    fs::symlink_metadata(path).is_ok_and(|m| m.is_file() && !m.file_type().is_symlink() && m.len() == asset.size)
        && hash_file(path).is_ok_and(|h| h == (asset.size, asset.sha256.clone()))
}
fn nonce() -> String {
    format!("{}-{}", std::process::id(), SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos())
}
fn sync_dir(path: &Path) -> Result<()> {
    #[cfg(not(windows))] { io(io(File::open(path))?.sync_all())?; }
    #[cfg(windows)] let _ = path;
    Ok(())
}
fn ensure_dir(path: &Path) -> Result<()> {
    if path.is_dir() { return Ok(()); }
    let parent = path.parent().ok_or("directory has no parent")?;
    ensure_dir(parent)?;
    io(fs::create_dir(path))?;
    sync_dir(parent)
}
fn replace(from: &Path, to: &Path) -> Result<()> {
    #[cfg(not(windows))] { io(fs::rename(from, to))?; }
    #[cfg(windows)] {
        use std::os::windows::ffi::OsStrExt;
        #[link(name = "kernel32")] extern "system" { fn MoveFileExW(a: *const u16, b: *const u16, flags: u32) -> i32; }
        let a: Vec<u16> = from.as_os_str().encode_wide().chain(Some(0)).collect();
        let b: Vec<u16> = to.as_os_str().encode_wide().chain(Some(0)).collect();
        if unsafe { MoveFileExW(a.as_ptr(), b.as_ptr(), 1 | 8) } == 0 { return Err(std::io::Error::last_os_error().to_string()); }
    }
    sync_dir(to.parent().ok_or("no parent")?)
}
fn atomic_write(path: &Path, write: impl FnOnce(&mut File) -> Result<()>) -> Result<()> {
    let parent = path.parent().ok_or("no parent")?;
    ensure_dir(parent)?;
    let temporary = parent.join(format!(".aethus-update-{}", nonce()));
    let mut file = io(OpenOptions::new().write(true).create_new(true).open(&temporary))?;
    let result = write(&mut file).and_then(|_| io(file.sync_all()));
    drop(file);
    if let Err(e) = result { let _ = fs::remove_file(&temporary); return Err(e); }
    replace(&temporary, path)
}
fn write_json(path: &Path, value: &Value) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?;
    atomic_write(path, |f| io(f.write_all(&bytes)))
}
fn copy_atomic(from: &Path, to: &Path) -> Result<()> {
    let mut source = io(File::open(from))?;
    atomic_write(to, |out| {
        // The title mount makes small writes very expensive. Match the HTTP transport's
        // large transfers instead of std::io::copy's small generic fallback buffer.
        let mut buffer = vec![0u8; 1024 * 1024];
        loop {
            let n = io(source.read(&mut buffer))?;
            if n == 0 { break; }
            io(out.write_all(&buffer[..n]))?;
        }
        #[cfg(unix)] {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = io(source.metadata())?.permissions();
            if to.file_name().is_some_and(|name| name == "eboot.bin") {
                permissions.set_mode(permissions.mode() | 0o111);
            }
            io(out.set_permissions(permissions))?;
        }
        Ok(())
    })
}
fn read_json(path: &Path) -> Result<Value> {
    read_optional_json(path)?.ok_or_else(|| format!("missing update state: {}", path.display()))
}
fn read_optional_json(path: &Path) -> Result<Option<Value>> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    let mut bytes = Vec::new();
    file.take(MANIFEST_LIMIT + 1).read_to_end(&mut bytes)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    if bytes.len() as u64 > MANIFEST_LIMIT { return Err("oversized update state".into()); }
    serde_json::from_slice(&bytes).map(Some).map_err(|e| format!("{}: {e}", path.display()))
}

pub struct Updater { pub root: PathBuf, pub state: PathBuf }
#[derive(Debug, PartialEq)]
pub enum Recovery { Clean, Trial, Restart }
impl Updater {
    pub fn new(root: &Path) -> Result<Self> {
        // Check userdata too: a symlink must not redirect the transaction outside the title.
        let state = checked_join(root, "userdata/updates")?;
        ensure_dir(&state)?;
        Ok(Self { root: root.into(), state })
    }
    pub fn installed(&self) -> Result<Value> {
        let path = self.state.join("installed.json");
        if path.exists() { read_json(&path) } else { Ok(Value::Null) }
    }
    pub fn check(&self, fetch: &mut impl Fetch, policy: &Policy) -> Result<Option<Manifest>> {
        let mut bytes = Vec::new();
        fetch.get(&policy.manifest_url, policy, &mut bytes, MANIFEST_LIMIT)?;
        let m = Manifest::parse(&bytes, policy)?;
        let installed = self.installed()?;
        if installed["id"].as_str() == Some(&m.id) { return Ok(None); }
        if installed["sequence"].as_u64().is_some_and(|s| m.sequence <= s) { return Err("refusing release downgrade or reused sequence".into()); }
        let rejected = self.state.join("rejected.json");
        if rejected.exists() && read_json(&rejected)?["id"].as_str() == Some(&m.id) {
            return Err("this release previously failed its startup check".into());
        }
        Ok(Some(m))
    }
    fn stage_dir(&self, m: &Manifest) -> Result<PathBuf> { checked_join(&self.state, &format!("staging/{}", m.id)) }
    pub fn stage(&self, m: &Manifest, policy: &Policy, fetch: &mut impl Fetch,
                 mut progress: impl FnMut(&str, usize, usize)) -> Result<()> {
        let stage = self.stage_dir(m)?;
        ensure_dir(&stage)?;
        for (i, a) in m.files.iter().enumerate() {
            progress(&a.path, i, m.files.len());
            let destination = checked_join(&self.root, &a.path)?;
            if matches(&destination, a) { continue; }
            let staged = checked_join(&stage, &a.path)?;
            if !matches(&staged, a) {
                atomic_write(&staged, |file| fetch.get(&a.url, policy, file, a.size))?;
                if !matches(&staged, a) { return Err(format!("download size/SHA-256 mismatch: {}", a.path)); }
            }
        }
        self.preflight(m)?;
        write_json(&self.state.join("ready.json"), &m.raw)
    }
    pub fn ready(&self) -> Result<Option<Manifest>> {
        let path = self.state.join("ready.json");
        if path.exists() { Ok(Some(Manifest::from_value(read_json(&path)?, None)?)) } else { Ok(None) }
    }
    fn preflight(&self, m: &Manifest) -> Result<Vec<Asset>> {
        let stage = self.stage_dir(m)?;
        let mut changed = Vec::new();
        for a in &m.files {
            let live = checked_join(&self.root, &a.path)?;
            let source = if matches(&live, a) { live } else {
                let p = checked_join(&stage, &a.path)?;
                if !matches(&p, a) { return Err(format!("staged file is missing or changed: {}", a.path)); }
                changed.push(a.clone()); p
            };
            if a.path == "eboot.bin" {
                let mut magic = [0; 4];
                io(io(File::open(&source))?.read_exact(&mut magic))?;
                if magic != 0x1d3d154fu32.to_le_bytes() { return Err("update eboot is not a PS5 SELF".into()); }
            } else if a.path == "sce_sys/param.json" && read_json(&source)?["titleId"] != "PPSA99320" {
                return Err("package title metadata mismatch".into());
            }
        }
        Ok(changed)
    }
    /// Called only after the host has stopped calling game code and flushed storage.
    pub fn install(&self, m: &Manifest) -> Result<()> { self.install_with(m, |_| Ok(())) }
    pub fn install_reporting(&self, m: &Manifest, progress: impl FnMut(&str, &str)) -> Result<()> {
        self.install_steps(m, |_| Ok(()), progress)
    }
    fn install_with(&self, m: &Manifest, mut checkpoint: impl FnMut(usize) -> Result<()>) -> Result<()> {
        self.install_steps(m, &mut checkpoint, |_, _| ())
    }
    fn install_steps(&self, m: &Manifest, mut checkpoint: impl FnMut(usize) -> Result<()>,
                     mut progress: impl FnMut(&str, &str)) -> Result<()> {
        if self.state.join("transaction.json").exists() { return Err("an update transaction already needs recovery".into()); }
        let changed = self.preflight(m)?;
        let transaction = format!("{}-{}", m.sequence, nonce());
        let backup = checked_join(&self.state, &format!("backups/{transaction}"))?;
        ensure_dir(&backup)?;
        let mut old = Vec::new();
        for a in &changed {
            progress("backup", &a.path);
            let live = checked_join(&self.root, &a.path)?;
            let saved = checked_join(&backup, &a.path)?;
            let original = if live.exists() {
                let (size, hash) = hash_file(&live)?;
                copy_atomic(&live, &saved)?;
                if hash_file(&saved)? != (size, hash.clone()) { return Err("backup verification failed".into()); }
                json!({"size": size, "sha256": hash})
            } else { Value::Null };
            old.push(json!({"path": a.path, "original": original}));
        }
        let mut journal = json!({"manifest": m.raw, "backup": transaction, "phase": "applying",
            "trial": false, "old": old, "previous": self.installed()?});
        // The durable journal and all verified originals precede the first live-file change.
        write_json(&self.state.join("transaction.json"), &journal)?;
        checkpoint(0)?;
        for (i, a) in changed.iter().enumerate() {
            progress("replace", &a.path);
            copy_atomic(&checked_join(&self.stage_dir(m)?, &a.path)?, &checked_join(&self.root, &a.path)?)?;
            if !matches(&self.root.join(&a.path), a) { return Err(format!("installed verification failed: {}", a.path)); }
            checkpoint(i + 1)?;
        }
        write_json(&self.state.join("installed.json"), &json!({"id": m.id, "version": m.version, "sequence": m.sequence}))?;
        journal["phase"] = json!("installed");
        write_json(&self.state.join("transaction.json"), &journal)?;
        Ok(())
    }
    pub fn recover(&self) -> Result<Recovery> {
        let path = self.state.join("transaction.json");
        if !path.exists() { return Ok(Recovery::Clean); }
        let mut journal = read_json(&path)?;
        let m = Manifest::from_value(journal["manifest"].clone(), None)?;
        if journal["phase"] == "installed" && journal["trial"] == false {
            journal["trial"] = json!(true);
            write_json(&path, &journal)?;
            return Ok(Recovery::Trial);
        }
        let backup_name = text(&journal, "backup")?;
        if !identifier(backup_name) { return Err("invalid backup identity".into()); }
        let backup = checked_join(&self.state, &format!("backups/{backup_name}"))?;
        let old = journal["old"].as_array().ok_or("invalid rollback list")?;
        // Validate every rollback input before touching anything, including on retry.
        let mut paths = HashSet::new();
        for entry in old {
            let name = text(entry, "path")?;
            if !payload_path(name) || !paths.insert(name) || !m.files.iter().any(|a| a.path == name) { return Err("invalid rollback path".into()); }
            checked_join(&self.root, name)?;
            if !entry["original"].is_null() {
                let size = number(&entry["original"], "size")?;
                let sha = text(&entry["original"], "sha256")?;
                if hash_file(&checked_join(&backup, name)?)? != (size, sha.into()) { return Err("rollback backup is damaged".into()); }
            }
        }
        for entry in old {
            let name = text(entry, "path")?;
            let live = checked_join(&self.root, name)?;
            if entry["original"].is_null() {
                if live.exists() { io(fs::remove_file(&live))?; sync_dir(live.parent().unwrap())?; }
            } else { copy_atomic(&checked_join(&backup, name)?, &live)?; }
        }
        write_json(&self.state.join("installed.json"), &journal["previous"])?;
        write_json(&self.state.join("rejected.json"), &json!({"id": m.id, "reason": "interrupted installation or unsuccessful first boot"}))?;
        if self.state.join("ready.json").exists() { io(fs::remove_file(self.state.join("ready.json")))?; }
        io(fs::remove_file(path))?;
        sync_dir(&self.state)?;
        Ok(Recovery::Restart)
    }
    pub fn confirm_boot(&self) -> Result<()> {
        let path = self.state.join("transaction.json");
        if path.exists() {
            let j = read_json(&path)?;
            if j["phase"] != "installed" || j["trial"] != true { return Err("update is not in its first boot".into()); }
            write_json(&self.state.join("last-success.json"), &j)?;
            io(fs::remove_file(path))?;
            if self.state.join("ready.json").exists() { io(fs::remove_file(self.state.join("ready.json")))?; }
            sync_dir(&self.state)?;
        }
        Ok(())
    }
    /// Only a confirmed release may discard older backups. Keep the single previous
    /// version named by last-success; an active transaction keeps all recovery inputs.
    pub fn cleanup_confirmed(&self) -> Result<()> {
        if self.state.join("transaction.json").try_exists().map_err(|e| e.to_string())? { return Ok(()); }
        let Some(success) = read_optional_json(&self.state.join("last-success.json"))? else { return Ok(()); };
        let keep = text(&success, "backup")?;
        if !identifier(keep) { return Err("invalid retained backup identity".into()); }
        let ready = self.ready()?.map(|m| m.id);
        for (directory, retained) in [("backups", Some(keep)), ("staging", ready.as_deref())] {
            let parent = checked_join(&self.state, directory)?;
            if !parent.try_exists().map_err(|e| e.to_string())? { continue; }
            for entry in io(fs::read_dir(&parent))? {
                let entry = io(entry)?;
                let name = entry.file_name();
                let name = name.to_str().ok_or("invalid update directory name")?;
                if retained == Some(name) { continue; }
                let path = checked_join(&parent, name)?;
                if io(entry.file_type())?.is_dir() { io(fs::remove_dir_all(path))?; }
            }
            sync_dir(&parent)?;
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "update/tests.rs"]
mod tests;
