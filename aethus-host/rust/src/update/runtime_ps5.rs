//! Background checks/downloads and controller-driven install. Startup recovery runs before JS.
use super::{read_json, Manifest, Policy, Recovery, Result, Updater};
use crate::log::{say, warn};
use std::path::Path;
use std::sync::mpsc::{self, Receiver};

const FEED: &str = "https://github.com/Sleepywalker69/azeroth-survivors-ps5/releases/download/ps5-update/update.json";
extern "C" { fn sceKernelSendNotificationRequest(device: i32, data: *const u8, size: usize, flags: i32) -> i32; }
pub fn notify(text: &str) {
    say!("update: {text}");
    // PS5_PayloadSDK/samples/hello_world's notification layout: message starts at byte 45.
    let mut request = [0u8; 3120];
    let bytes = text.as_bytes();
    let n = bytes.len().min(3074);
    request[45..45 + n].copy_from_slice(&bytes[..n]);
    unsafe { sceKernelSendNotificationRequest(0, request.as_ptr(), request.len(), 0); }
}

pub struct Service {
    updater: Updater,
    policy: Option<Policy>,
    messages: Option<Receiver<Result<Option<Manifest>>>>,
    ready: Option<Manifest>,
    started: bool,
    trial: bool,
    healthy_frames: usize,
}
impl Service {
    /// A true return requests closing the title: an executable was replaced or recovered.
    pub fn open(root: &Path) -> Result<(Self, bool)> {
        let updater = Updater::new(root)?;
        let recovery = updater.recover()?;
        if recovery == Recovery::Restart { notify("Aethus: the previous version was restored. Please launch again."); }
        let config_path = root.join("userdata/updater.json");
        let policy = match (|| {
            let config = if config_path.exists() { read_json(&config_path)? } else { serde_json::Value::Null };
            if config["enabled"] == false { Ok(None) } else {
                Policy::new(config["manifest_url"].as_str().unwrap_or(FEED), config["manifest_sha256"].as_str()).map(Some)
            }
        })() {
            Ok(p) => p,
            Err(e) => { warn!("update: invalid configuration: {e}"); notify("Aethus update settings need attention. The game can still run."); None }
        };
        let mut service = Self { updater, policy, messages: None, ready: None, started: false,
            trial: recovery == Recovery::Trial, healthy_frames: 0 };
        if recovery == Recovery::Restart { return Ok((service, true)); }
        // A fully downloaded update from the preceding session installs on the next launch.
        // Never install over a trial that has not yet reached the game successfully.
        if recovery == Recovery::Clean && service.policy.is_some() {
            if let Some(m) = service.updater.ready()? {
                if service.updater.installed()?["id"].as_str() != Some(&m.id) {
                    service.ready = Some(m);
                    service.install_and_close()?;
                    return Ok((service, true));
                }
            }
        }
        Ok((service, false))
    }
    fn start(&mut self) {
        if self.messages.is_some() || self.policy.is_none() { return; }
        let root = self.updater.root.clone();
        let policy = self.policy.clone().unwrap();
        let (tx, rx) = mpsc::channel();
        self.messages = Some(rx);
        self.started = true;
        let spawn = std::thread::Builder::new().name("aethus-updater".into()).stack_size(2 * 1024 * 1024).spawn(move || {
            let result = (|| {
                let updater = Updater::new(&root)?;
                let mut http = super::net_ps5::Http::new()?;
                say!("update: checking {}", policy.manifest_url);
                let Some(manifest) = updater.check(&mut http, &policy)? else { return Ok(None); };
                notify(&format!("Aethus {} is downloading in the background.", manifest.version));
                updater.stage(&manifest, &policy, &mut http, |path, i, count| {
                    say!("update: checking/downloading file {}/{count}: {path}", i + 1);
                })?;
                Ok(Some(manifest))
            })();
            let _ = tx.send(result);
        });
        if let Err(e) = spawn { self.messages = None; warn!("update: worker: {e}"); }
    }
    /// Main thread only. No file hashing or download happens on the frame loop.
    pub fn poll(&mut self, game_ready: bool, drew_frame: bool, action: bool) -> bool {
        if game_ready && drew_frame { self.healthy_frames += 1; }
        if self.healthy_frames >= 120 {
            if self.trial {
                match self.updater.confirm_boot() {
                    Ok(()) => { self.trial = false; notify("Aethus update started successfully. Your previous version remains backed up."); }
                    Err(e) => { warn!("update: could not confirm first boot: {e}"); return false; }
                }
            }
            if !self.started { self.start(); }
        }
        if let Some(rx) = self.messages.as_ref() {
            match rx.try_recv() {
                Ok(Ok(Some(m))) => {
                    notify(&format!("Aethus {} is ready. Hold L3 + Options for 2 seconds to install and close, or install on next launch.", m.version));
                    self.ready = Some(m); self.messages = None;
                }
                Ok(Ok(None)) => { say!("update: already current"); self.messages = None; }
                Ok(Err(e)) => {
                    warn!("update: {e}");
                    notify("Aethus could not check or download an update. The current game is unchanged. Hold L3 + Options to retry.");
                    self.messages = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => { self.messages = None; warn!("update: worker ended without a result"); }
                Err(mpsc::TryRecvError::Empty) => (),
            }
        }
        if action {
            if self.trial { notify("Aethus: finishing the new version's startup check."); }
            else if self.ready.is_some() { return true; }
            else if self.policy.is_none() { notify("Aethus updates are disabled in updater.json."); }
            else if self.messages.is_some() { notify("Aethus update check or download is still running."); }
            else { notify("Aethus: checking for updates."); self.start(); }
        }
        false
    }
    pub fn install_and_close(&mut self) -> Result<()> {
        let m = self.ready.take().ok_or("no verified update is ready")?;
        notify(&format!("Installing Aethus {}. The game will close when finished.", m.version));
        match self.updater.install(&m) {
            Ok(()) => { notify("Aethus update installed. Please launch the game again."); Ok(()) }
            Err(e) => {
                warn!("update: installation failed: {e}");
                match self.updater.recover() {
                    Ok(_) => notify("Aethus update failed; the previous version is intact. Please launch again."),
                    Err(recovery) => return Err(format!("update failed: {e}; recovery needs attention: {recovery}")),
                }
                Ok(())
            }
        }
    }
}
