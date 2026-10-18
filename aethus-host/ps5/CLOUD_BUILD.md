# GitHub builder for PS5 updates

The workflow checks the Aethus website every six hours, compares web/patch identity and
host source with the published channel, and compiles only when something changes. It runs
on GitHub's Ubuntu 24.04 runner. The user's PC is not involved after activation.

This pipeline is prepared for `Sleepywalker69/azeroth-survivors-ps5`. The maintained source
branch is `ps5-port`. GitHub schedules require the workflow file on the default branch,
`main`, so the scheduler is delivered through a pull request. Automatic execution is also
guarded by the repository variable `AETHUS_CLOUD_ENABLED=true`. Until that variable is set
and the pinned build kit is published, the scheduled publisher is inactive.

## Inputs and boundaries

`tools/cloud-kit.json` pins the exact size and SHA-256 of a toolkit made from the existing
working Linux inputs: installed PS5 SDK, RADV archive/link tools/runtime, Rust 1.98.1 plus
rust-src, offline crate/index cache, QuickJS-ng, and native WABT 1.0.42. The kit contains no
game files, saves, credentials, Cargo credentials/configuration or user shell profiles.
Its inventory hashes every file. Extraction verifies the archive first, rejects archive
escapes and special files, then verifies every extracted entry and executable mode.

The toolkit provenance is:

- Installed SDK: `611893fc25ef3e138f5bc2c90d829d8b58174cd2`,
  [PS5_PayloadSDK source](https://github.com/mihawk-99/PS5_PayloadSDK).
- Vulkan/link runtime: `f410e67a0241b3e8edcdbcd2a9b285dfc4a412ed`,
  [PS5_Vulkan source](https://github.com/mihawk-99/PS5_Vulkan).
- RADV archive: Mesa `1ba9874f478657ccf6da72ce55f0260164cc0bd0`,
  [PS5_Mesa source](https://github.com/mihawk-99/PS5_Mesa). Its own provenance records
  the earlier SDK used when that archive was built; the title uses the installed SDK above.
- Native wasm2c: WABT 1.0.42, built from the cached source with a Linux-generated config.
  Rust's included source already contains the documented PS5 std patches.

Each new candidate downloads only `wenilla_bg.wasm` (256 MiB limit), `wenilla.js` and its
referenced snippets (2 MiB each), bounded JSON metadata, and missing patch-2 onward parts
listed by the Aethus manifest. Patch parts and reconstructed MPQs have size limits, and the
final MPQs must match upstream hashes. No Windows game executable is downloaded. Existing
base game MPQs and `patch.MPQ` remain installation inputs and are preserved on the console.
Rust builds remain offline. Ubuntu LLVM 18 packages are installed only if missing.

Unknown or changed executable glue imports/snippets stop the build for review. Automation
does not promise compatibility with arbitrary future browser API or game architecture changes.
All Python tools run with `-I`; downloaded JavaScript is parsed, not executed by the publisher.

## Release sequence

1. Validate upstream metadata, download the candidate, and check the reviewed host interface.
2. Translate using native Linux wasm2c, compile C/Rust, link, and validate the PS5 SELF.
3. Prepare a content-addressed feed. Unchanged payloads reuse previously published immutable
   release URLs. The console checks each required file and downloads only changed content.
4. Upload new blobs to a unique draft release. Verify GitHub's SHA-256 and size for every
   new/reused asset before publishing that release.
5. Upload and verify the channel manifest under a temporary name, retain the previous
   channel pointer, then rename the new pointer to `update.json`. Failure to rename attempts
   to restore the previous pointer. No blob asset is overwritten.

The channel URL is the default in the host:

```
https://github.com/Sleepywalker69/azeroth-survivors-ps5/releases/download/ps5-update/update.json
```

Changing from a pinned LAN test requires replacing `userdata/updater.json` with this HTTPS
URL and removing the LAN manifest pin. It takes effect at the next title launch. The title
downloads in the background; hold L3 + Options to install and close, then launch again.
The console retains one previous-version backup after successful startup.

## Test and activation

The workflow supports manual runs with `publish=false` to keep a candidate artifact without
changing the console channel. `force=true` rebuilds the same upstream version for host fixes.
Build logs and plans are retained for 14 days; unpublished candidate artifacts for 3 days.
GitHub release history is separate from the console's single-backup policy.

Before activation: complete the isolated Linux build, publish the pinned toolkit, merge the
scheduler PR, run an unpublished GitHub build, verify its candidate, then publish and test
one console update. Enable the scheduled publisher only after that result is accepted.

The workflow has one publisher at a time. Its GitHub token is exposed only to the publishing
step, after compilation and verification. The checkout does not retain Git credentials.
An upstream compatibility failure leaves the current channel and running game unchanged.
GitHub may delay scheduled runs, and public-repository schedules can be disabled after
60 days without repository activity; this is a GitHub scheduler limitation.

References: [scheduled workflow behavior](https://docs.github.com/en/actions/reference/workflows-and-actions/events-that-trigger-workflows#schedule),
[release asset digests](https://docs.github.com/en/rest/releases/assets).
