// The page around the client, for the Aethus host.
//
// On wow.aethus.pl, rogue.js sets the client's page settings and boots the wasm through boot.js
// (a loading overlay and an HTTP-cache prefetch), with addons.js (the player's AddOns folder, kept
// in IndexedDB) and saves.js (save export and import through file pickers) beside it. This module
// does the same for the host: the same settings, the same window hooks and the same init call,
// with the overlay left out and the player's files on the embedder's disk.
//
// The embedder's operations used here:
//   progress(stage)            the client's boot stage ('startup', then 'ready')
//   fatal(message)             the client could not start
//   fs_read(rel) -> bytes or null; fs_list(rel) -> JSON [name as written, ...];
//   fs_kind(rel) -> 'file', 'dir' or ''
//       `rel` is a lowercased path under the AddOns folder ('' for the folder itself), which
//       the embedder resolves without regard to case.
//   addon_remove(name)         deletes an installed addon's folder
//   save_export(text) -> path  writes the sealed save to a new file
//   save_pick() -> JSON { name, text } or null: the save file the player put out for import
const { config, rust, queueTask } = globalThis.__aethus;

// ── rogue.js: the client's page settings ─────────────────────────────────────────────────────

// The query string's quick-start keys, from the host's configuration instead.
const rogue = config.rogue || {};
window.__rogue_config = Object.fromEntries(
  ['glue', 'bg', 'class', 'race', 'name', 'gender'].map((k) => [k, rogue[k] == null ? '' : String(rogue[k])])
);
window.__wenilla_data_layout = 'static';
window.__wenilla_data_base = new URL('./data', location.href).href;
// The page's low-cost default for a one-thread client; the host's configuration may add CVars.
window.__wenilla_env = { farclip: '420', ...(config.env || {}) };

// ── boot.js: the client's progress calls ─────────────────────────────────────────────────────

window.__wenilla_progress = (stage) => {
  console.log(`client stage: ${stage}`);
  try {
    rust('progress', String(stage));
  } catch (e) {
    console.warn('progress:', e);
  }
};

// ── addons.js: the AddOns folder ─────────────────────────────────────────────────────────────

// The client's AddOns folder: local_state's home plus `AddOns`.
const ADDONS_ROOT = '/benilla-config/addons';

// A client path under the AddOns root, lowercased and relative; null when it is not under it.
function rel(path) {
  const p = String(path).replace(/\\/g, '/').replace(/\/+$/, '').toLowerCase();
  if (p === ADDONS_ROOT) return '';
  return p.startsWith(`${ADDONS_ROOT}/`) ? p.slice(ADDONS_ROOT.length + 1) : null;
}

// The client's synchronous reads (benilla_formats::host_fs).
window.__wenilla_fs = {
  read(path) {
    const r = rel(path);
    if (r === null) return undefined;
    return rust('fs_read', r) || undefined;
  },
  list(path) {
    const r = rel(path);
    if (r === null) return [];
    const text = rust('fs_list', r);
    return text ? JSON.parse(text) : [];
  },
  kind(path) {
    const r = rel(path);
    if (r === null) return undefined;
    return rust('fs_kind', r) || undefined;
  },
};

// The Options screen's AddOns page. There is no file picker here: an addon is installed by
// copying its folder into the AddOns folder, and it loads at the next run.
const addonState = {
  busy: false,
  message: '',
  // Bumped on every change, so the screen knows to read the list again.
  rev: 0,
};
window.__rogue_addons = {
  state: addonState,
  install() {
    addonState.message = config.addonsDir
      ? `To install an addon, copy its folder into ${config.addonsDir}. It loads at your next run.`
      : 'Addons cannot be installed on this system.';
    addonState.rev++;
  },
  remove(name) {
    if (addonState.busy) return;
    try {
      rust('addon_remove', String(name));
      addonState.message = `Removed ${name}.`;
    } catch (e) {
      addonState.message = `Something went wrong: ${e && e.message ? e.message : e}`;
    }
    addonState.rev++;
  },
};

// ── saves.js: the save as a file ─────────────────────────────────────────────────────────────

const saveState = {
  // Bumped when a picked file is ready for the screen to take.
  rev: 0,
};
let picked = null;
window.__rogue_saves = {
  state: saveState,
  download(text) {
    try {
      console.log(`The save was written to ${rust('save_export', String(text))}`);
    } catch (e) {
      console.warn('The save could not be written:', e);
    }
  },
  pick() {
    let file = null;
    try {
      const text = rust('save_pick');
      file = text ? JSON.parse(text) : null;
    } catch (e) {
      console.warn('The save file could not be read:', e);
    }
    if (!file) return;
    // As from a file picker: the file is ready in a later task.
    queueTask(() => {
      picked = { name: String(file.name), text: String(file.text) };
      saveState.rev++;
    });
  },
  // The picked file, `{ name, text }`, once; null when there is none.
  take() {
    const p = picked;
    picked = null;
    return p;
  },
};

// ── WebAssembly: the one module, translated ahead of time ────────────────────────────────────
//
// The glue only instantiates: a Module stands for the translation (its bytes, if given, are
// not read) and an Instance's exports are the translation's.

class Module {
  static exports() {
    return [];
  }
  static imports() {
    return [];
  }
  static customSections() {
    return [];
  }
}
class Instance {
  constructor(module, importObject) {
    this.exports = globalThis.__native.instantiate(importObject);
  }
}
class CompileError extends Error {}
class LinkError extends Error {}
class RuntimeError extends Error {}
for (const E of [CompileError, LinkError, RuntimeError]) Object.defineProperty(E.prototype, 'name', { value: E.name });

globalThis.__aethus.define(globalThis, 'WebAssembly', {
  Module,
  Instance,
  CompileError,
  LinkError,
  RuntimeError,
  validate: () => true,
  compile: async () => new Module(),
  instantiate: async (source, importObject) => {
    if (source instanceof Module) return new Instance(source, importObject);
    const module = new Module();
    return { module, instance: new Instance(module, importObject) };
  },
});

// ── Boot ─────────────────────────────────────────────────────────────────────────────────────

try {
  const { default: init } = await import('../web/wenilla.js');
  await init({ module_or_path: new Module() });
  document.getElementById('benilla').focus();
} catch (e) {
  console.error('The game failed to start:', e);
  try {
    rust('fatal', String(e && e.message ? e.message : e));
  } catch (x) {
    console.error('fatal:', x);
  }
}
