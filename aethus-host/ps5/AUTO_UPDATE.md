# PS5 updates

The host checks for a prepared PS5 release after the game starts. Downloads and hashing run
on another thread. Play can continue while files are staged. A notification announces an
available download and another announces that it is ready.

Hold **L3 + Options for two seconds** to install a ready update and close the title. Launch
the title again to play the new version. The same chord checks again after a network failure.
If an update is already staged when the title starts, it installs before loading the game and
closes; launch once more. Saves, AddOns, `env.txt`, the import folder and base MPQs are preserved.

The console does not need a running PC once a release is hosted. The publisher must compile
each upstream web version because wasm2c links that game into `eboot.bin`. A new upstream
JavaScript/WASM download on its own is not a PS5 executable.

## Feed configuration

The title accesses its installed folder through `/app0`, including
`/app0/userdata/updater.json`. FTP exposes that same title folder at
`/data/homebrew/PPSA99320`. `AETHUS_INSTALL_ROOT` can override the title-side root.

`userdata/updater.json` is optional. The default manifest URL is:

```
https://github.com/Sleepywalker69/azeroth-survivors-ps5/releases/download/ps5-update/update.json
```

This channel is live and publishes verified PS5 builds. The current release is
[Aethus 0.98.0](https://github.com/Sleepywalker69/azeroth-survivors-ps5/releases/tag/ps5-0.98.0-20261018162554).
Builds are started manually in GitHub Actions; there is no scheduled website check.
See [CLOUD_BUILD.md](CLOUD_BUILD.md). Override the feed for another HTTPS host:

```json
{"enabled":true,"manifest_url":"https://example.com/ps5/update.json"}
```

Set `enabled` to `false` to stop checks/downloads/automatic installation of staged updates.
An interrupted installation still needs recovery even when checking is disabled.

For a private LAN test, HTTP is accepted only for a private/loopback IPv4 address and only
with the exact `manifest_sha256` stored in this local configuration. The manifest pins every
payload's size and SHA-256. Changing the LAN test feed requires updating its manifest pin.
Production HTTPS checks certificate trust, hostname and validity dates. Redirects are bounded
and restricted to the configured host; GitHub release-asset redirects are supported.

Older hosts failed on GitHub's roughly 5.2 KB redirect headers with `0x80431073`
(`TOO_LARGE_RESPONSE_HEADER`). The 0.98.0 host configures a bounded 64 KiB response-header
limit and logs response status and redirect hops. TLS certificate checks remain enabled.
An affected older host needs the manual 0.98.0 package once before it can fetch GitHub
updates. Close the title before replacing files and preserve executable mode 0755.

## Preparing a release

From PowerShell in the active host's parent directory:

```powershell
aethus-host\tools\update-pc.cmd --web C:\path\to\reviewed-web-build --build --cpu zen2 `
  --data C:\path\to\patches --feed-output C:\path\to\new-feed-directory `
  --feed-base-url https://example.com/ps5/releases/unique-release `
  --sequence 2026101801
```

`--web` is offline. Existing download/compatibility checks also work with the explicit
`--download` path. The sequence must increase for each console release, including host fixes
that keep the same game version. Feed preparation writes immutable hash-named blobs and
`update.json`; it does not upload or publish anything. Publish all blobs first, and replace the
channel manifest last. A GitHub channel can use immutable release-tag URLs for blobs and the
`ps5-update` release's `update.json` as its current manifest. Do not overwrite old blob assets.

For an already built title, `python -I aethus-host/tools/package_update.py --help` lists the
standalone packaging options. `serve_update.py` serves only the manifest and hash-named blobs
for a pinned LAN test; it exposes no directory listing or other workspace files.

## Installation and recovery

The console rejects wrong titles, malformed paths, symlinks, duplicate paths, downgrades,
oversized responses, incomplete releases and wrong hashes. It verifies every required file
before changing the installed game. Files that already match are retained.

Before installation, each changed original is copied, flushed and verified under
`userdata/updates/backups/`. A flushed transaction journal precedes any live replacement.
Each replacement uses a same-directory temporary file and atomic rename. The executable is
replaced last. A process interruption leaves a journal that the next host startup uses to
restore the previous files. New files absent from the old version are removed during rollback.

The first launch after installation is a trial. Reaching the client's ready stage and 120
frame callbacks confirms it. If the next launch finds an unconfirmed trial, it restores the
previous version and closes. That failed release is recorded so it is not immediately retried.
After confirmation, the host keeps only the backup of the previous version and removes older
update backups and consumed staging files. An active installation or unconfirmed first boot
keeps its recovery files. A separately staged next update is retained. This policy applies to
the console updater's `userdata/updates/` folders; FTP development deployment archives are
separate. These checks cannot repair a new executable that fails before the host's updater
code can run; its backup can be restored over FTP.

Large local copies use 1 MiB buffers and preserve executable permissions. The original
installer's small-buffer copy was measured at roughly 240 KiB/s on the title mount, making
the 262 MB executable replacement take about 18 minutes after its backup. The larger-buffer
implementation passes offline copy/transaction tests. The measured 0.97.0 to 0.98.0 update
took **7.280 seconds** for backup, installation and verification, including the 262 MB
executable. Checking GitHub, downloading changed files and staging verification took
**57.828 seconds** before installation. Logs were captured across relaunch. Notifications
distinguish backing up from installing, and the log records each file and phase.

Receipts and state are in `userdata/updates/`; runtime progress/errors are in
`userdata/aethus-host.log`. The tests inject interruption after each live-file replacement,
exercise first-boot rollback, and verify that saves and configuration stay unchanged.

## Validation commands

```bash
cargo test --offline --manifest-path aethus-host/test/updater/Cargo.toml
python3 -I aethus-host/test/test_package_update.py
```

The isolated Rust test crate uses the actual transaction implementation without the game or
PS5 libraries. Console download, installation and first-boot confirmation of 0.97.0 have
been verified. That first installer dropped executable bits; restoring mode 0755 fixed the
launch rejection, and the current host preserves them. The current host has also cleared
consumed staging and retained exactly one previous backup on console. A larger-buffer update
to 0.98.0 completed in 7.280 seconds. The production GitHub HTTPS download followed 302 to
200 responses, installed successfully and confirmed first boot. Independent FTP readback
matched all 20 release files to the published manifest; executable mode was 0755, exactly
one previous backup remained, and no transaction or ready-update marker remained.
The networking ABI references are the local PS5_PayloadSDK examples and the
[OpenOrbis HTTP declarations](https://github.com/OpenOrbis/OpenOrbis-PS4-Toolchain/blob/master/include/orbis/Http.h).
