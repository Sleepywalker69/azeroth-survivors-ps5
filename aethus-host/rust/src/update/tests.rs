use super::*;
use std::collections::HashMap;

struct Fixture { root: PathBuf, updater: Updater, policy: Policy, manifest: Manifest, fetch: MemoryFetch }
struct MemoryFetch { files: HashMap<String, Vec<u8>> }
impl Fetch for MemoryFetch {
    fn get(&mut self, url: &str, p: &Policy, out: &mut dyn Write, limit: u64) -> Result<()> {
        p.check_url(url, false)?;
        let body = self.files.get(url).ok_or("missing response")?;
        if body.len() as u64 > limit { return Err("response exceeds limit".into()); }
        io(out.write_all(body))
    }
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("aethus-updater-test-{}", nonce()));
        fs::create_dir_all(root.join("userdata")).unwrap();
        fs::create_dir_all(root.join("js")).unwrap();
        fs::write(root.join("userdata/localStorage.json"), b"save stays unchanged").unwrap();
        fs::write(root.join("env.txt"), b"settings stay unchanged").unwrap();
        fs::write(root.join("eboot.bin"), b"old executable").unwrap();
        fs::write(root.join("js/boot.mjs"), b"old boot").unwrap();
        let entries = [
            ("eboot.bin", [0x1d3d154fu32.to_le_bytes().as_slice(), b"new executable"].concat()),
            ("js/boot.mjs", b"new boot".to_vec()),
            ("web/wenilla.js", b"new web".to_vec()),
            ("sce_sys/param.json", br#"{"titleId":"PPSA99320"}"#.to_vec()),
        ];
        let mut files = HashMap::new();
        let mut assets = Vec::new();
        for (path, body) in entries {
            let url = format!("https://example.com/{path}");
            assets.push(json!({"path":path,"url":url,"size":body.len(),"sha256":hash_bytes(&body)}));
            files.insert(url, body);
        }
        let raw = json!({"format":1,"title_id":"PPSA99320","id":"test-2","version":"0.97.0","sequence":2,"files":assets});
        let policy = Policy::new("https://example.com/update.json", None).unwrap();
        let bytes = serde_json::to_vec(&raw).unwrap();
        files.insert(policy.manifest_url.clone(), bytes.clone());
        let manifest = Manifest::parse(&bytes, &policy).unwrap();
        let updater = Updater::new(&root).unwrap();
        Self { root, updater, policy, manifest, fetch: MemoryFetch { files } }
    }
    fn stage(&mut self) { self.updater.stage(&self.manifest, &self.policy, &mut self.fetch, |_,_,_| ()).unwrap(); }
    fn check_originals(&self) {
        assert_eq!(fs::read(self.root.join("eboot.bin")).unwrap(), b"old executable");
        assert_eq!(fs::read(self.root.join("js/boot.mjs")).unwrap(), b"old boot");
        assert!(!self.root.join("web/wenilla.js").exists());
        self.check_saves();
    }
    fn check_saves(&self) {
        assert_eq!(fs::read(self.root.join("userdata/localStorage.json")).unwrap(), b"save stays unchanged");
        assert_eq!(fs::read(self.root.join("env.txt")).unwrap(), b"settings stay unchanged");
    }
}
impl Drop for Fixture { fn drop(&mut self) { let _ = fs::remove_dir_all(&self.root); } }

#[test]
fn complete_install_is_verified_then_confirmed() {
    let mut f = Fixture::new();
    assert!(f.updater.check(&mut f.fetch, &f.policy).unwrap().is_some());
    f.stage();
    f.updater.install(&f.manifest).unwrap();
    for a in &f.manifest.files { assert!(matches(&f.root.join(&a.path), a)); }
    assert_eq!(f.updater.recover().unwrap(), Recovery::Trial);
    f.updater.confirm_boot().unwrap();
    assert_eq!(f.updater.recover().unwrap(), Recovery::Clean);
    assert!(f.updater.check(&mut f.fetch, &f.policy).unwrap().is_none());
    assert!(f.updater.state.join("last-success.json").exists());
    f.check_saves();
}
#[test]
fn interruption_after_every_live_replacement_recovers_originals() {
    for stop in 0..=4 {
        let mut f = Fixture::new(); f.stage();
        assert!(f.updater.install_with(&f.manifest, |i| if i == stop { Err("simulated power loss".into()) } else { Ok(()) }).is_err());
        assert_eq!(f.updater.recover().unwrap(), Recovery::Restart);
        f.check_originals();
        assert_eq!(f.updater.recover().unwrap(), Recovery::Clean);
        assert!(f.updater.check(&mut f.fetch, &f.policy).is_err());
    }
}
#[test]
fn failed_first_boot_rolls_back() {
    let mut f = Fixture::new(); f.stage(); f.updater.install(&f.manifest).unwrap();
    assert_eq!(f.updater.recover().unwrap(), Recovery::Trial);
    assert_eq!(f.updater.recover().unwrap(), Recovery::Restart);
    f.check_originals();
}
#[test]
fn corrupt_download_and_stage_cannot_modify_live_files() {
    let mut f = Fixture::new();
    f.fetch.files.get_mut("https://example.com/js/boot.mjs").unwrap()[0] ^= 1;
    assert!(f.updater.stage(&f.manifest, &f.policy, &mut f.fetch, |_,_,_| ()).unwrap_err().contains("SHA-256"));
    f.check_originals();
    let mut f = Fixture::new(); f.stage();
    fs::write(f.updater.stage_dir(&f.manifest).unwrap().join("eboot.bin"), b"bad").unwrap();
    assert!(f.updater.install(&f.manifest).is_err());
    assert!(!f.updater.state.join("transaction.json").exists());
    f.check_originals();
}
#[test]
fn damaged_rollback_input_stops_before_any_restore() {
    let mut f = Fixture::new(); f.stage();
    assert!(f.updater.install_with(&f.manifest, |i| if i == 4 { Err("power loss".into()) } else { Ok(()) }).is_err());
    let j = read_json(&f.updater.state.join("transaction.json")).unwrap();
    fs::write(f.updater.state.join("backups").join(j["backup"].as_str().unwrap()).join("eboot.bin"), b"bad").unwrap();
    assert!(f.updater.recover().unwrap_err().contains("damaged"));
    assert!(matches(&f.root.join("js/boot.mjs"), f.manifest.files.iter().find(|a| a.path == "js/boot.mjs").unwrap()));
}
#[test]
fn rejects_paths_aliases_collisions_and_wrong_title() {
    let f = Fixture::new();
    for path in ["../eboot.bin","/eboot.bin","js/../../env.txt","userdata/localStorage.json","env.txt","Data/base.MPQ","js/a\\b","js/a:b","js/a%2fb","js/./a","js/a."] {
        let mut v = f.manifest.raw.clone(); v["files"][0]["path"] = json!(path);
        assert!(Manifest::from_value(v, Some(&f.policy)).is_err(), "{path}");
    }
    let mut v = f.manifest.raw.clone(); let mut duplicate = v["files"][0].clone();
    duplicate["path"] = json!(duplicate["path"].as_str().unwrap().to_uppercase());
    v["files"].as_array_mut().unwrap().push(duplicate);
    assert!(Manifest::from_value(v, Some(&f.policy)).is_err());
    let mut v = f.manifest.raw.clone(); v["title_id"] = json!("PPSA00000");
    assert!(Manifest::from_value(v, Some(&f.policy)).is_err());
    let mut v = f.manifest.raw.clone();
    let boot = v["files"].as_array_mut().unwrap().iter_mut().find(|a| a["path"] == "js/boot.mjs").unwrap();
    boot["path"] = json!("js/BOOT.MJS");
    assert!(Manifest::from_value(v, Some(&f.policy)).is_err());
}
#[test]
fn untrusted_transport_and_manifest_pin_are_enforced() {
    assert!(Policy::new("http://example.com/update.json", None).is_err());
    let f = Fixture::new(); let bytes = serde_json::to_vec(&f.manifest.raw).unwrap();
    let pin = hash_bytes(&bytes);
    let p = Policy::new("http://192.168.0.20:8840/update.json", Some(&pin)).unwrap();
    assert!(p.check_url("http://192.168.0.20:8840/asset", false).is_ok());
    assert!(p.check_url("http://192.168.0.21:8840/asset", true).is_err());
    let p = Policy::new("https://example.com/update.json", Some(&"0".repeat(64))).unwrap();
    assert!(Manifest::parse(&bytes, &p).unwrap_err().contains("SHA-256"));
    let p = Policy::new("https://github.com/a/b/releases/download/x/update.json", None).unwrap();
    assert!(p.check_url("https://release-assets.githubusercontent.com/asset", true).is_ok());
    assert!(p.check_url("https://release-assets.githubusercontent.com/asset", false).is_err());
    assert!(p.check_url("http://github.com/asset", true).is_err());
    assert!(p.check_url("https://github.com.evil.test/asset", true).is_err());
    assert!(p.check_url("https://github.com@evil.test/asset", true).is_err());
}
#[test]
fn release_order_and_bounded_responses() {
    let mut f = Fixture::new();
    write_json(&f.updater.state.join("installed.json"), &json!({"id":"future","sequence":3})).unwrap();
    assert!(f.updater.check(&mut f.fetch, &f.policy).unwrap_err().contains("downgrade"));
    let mut f = Fixture::new();
    f.fetch.files.get_mut("https://example.com/eboot.bin").unwrap().extend_from_slice(b"oversize");
    assert!(f.updater.stage(&f.manifest, &f.policy, &mut f.fetch, |_,_,_| ()).is_err());
    f.check_originals();
}
#[cfg(unix)]
#[test]
fn symlinks_cannot_redirect_staging_or_installation() {
    let mut f = Fixture::new();
    std::os::unix::fs::symlink(f.root.join("userdata"), f.root.join("web")).unwrap();
    assert!(f.updater.stage(&f.manifest, &f.policy, &mut f.fetch, |_,_,_| ()).unwrap_err().contains("symlink"));
    f.check_saves();
}
