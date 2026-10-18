"""Generate equivalence tests for the actual host's QuickJS text encoder (Python -I)."""
import importlib.util
import sys
from pathlib import Path

spec = importlib.util.spec_from_file_location('prepare_glue', Path(__file__).resolve().parents[1] / 'tools/prepare_glue.py')
glue = importlib.util.module_from_spec(spec)
spec.loader.exec_module(glue)

for invalid in ['', glue.ORIGINAL * 2, glue.ORIGINAL.replace('arg.length', 'arg.length + 1', 1)]:
    try:
        glue.optimize(invalid)
    except ValueError:
        pass
    else:
        raise AssertionError('unreviewed helper accepted')
known = glue.ORIGINAL + '\n' + glue.ORIGINAL_GET + '\n' + glue.ORIGINAL_DECODE
assert glue.optimize('prefix\n' + known + '\nsuffix') == ('prefix\n' + glue.OPTIMIZED + '\n' + glue.OPTIMIZED_GET + '\n' + glue.ORIGINAL_DECODE + '\nsuffix')
try:
    glue.optimize(known.replace('numBytesDecoded += len;', 'numBytesDecoded += len + 1;'))
except ValueError:
    pass
else:
    raise AssertionError('unreviewed decoder accepted')

script = r'''
const cachedTextEncoder = {
  encode,
  encodeInto(s, view) { const r = encodeInto(s, view); return {read:r[0], written:r[1]}; }
};
let mem, next, WASM_VECTOR_LEN, reallocations, growOnMalloc;
function reset() { mem = new Uint8Array(16 << 20); setMemory(mem); next = 8; WASM_VECTOR_LEN = 0; reallocations = 0; }
function getUint8ArrayMemory0() { return mem; }
function malloc(n, alignment) {
  if (alignment !== 1) throw Error('wrong alignment');
  if (growOnMalloc) { const old = mem; mem = new Uint8Array(old.length * 2); mem.set(old); setMemory(mem); growOnMalloc = false; }
  const p = next; next += Math.max(n, 1); return p;
}
function realloc(p, old, n, alignment) {
  reallocations++;
  const q = malloc(n, alignment); mem.copyWithin(q, p, p + Math.min(old, n)); return q;
}
function check(fn, input, suppliedRealloc, grow) {
  reset(); growOnMalloc = grow;
  const p = fn(input, malloc, suppliedRealloc ? realloc : undefined);
  return Array.from(mem.subarray(p, p + WASM_VECTOR_LEN)).join(',');
}
'''
script += glue.ORIGINAL.replace('function passStringToWasm0(', 'function original(') + '\n'
script += glue.OPTIMIZED.replace('function passStringToWasm0(', 'function optimized(') + '\n'
script += glue.OPTIMIZED_GET + '\n'
script += r'''
const cases = ['', 'a', 'texture-compression-bc', 'hello\0world', '\u00e9\u4e2d\ud83d\ude00',
  '\ud800', '\udc00', 'x\ud800y', '\ufeffhello', 'x'.repeat(65537)];
for (const input of cases) {
  for (const suppliedRealloc of [false, true]) {
    for (const grow of [false, true]) {
      const a = check(original, input, suppliedRealloc, grow);
      const b = check(optimized, input, suppliedRealloc, grow);
      if (a !== b) throw Error('UTF-8 mismatch for ' + JSON.stringify(input));
    }
  }
}
for (const fn of [original, optimized]) {
  reset(); let threw = false;
  try { fn('abc', () => { throw Error('malloc trap'); }, realloc); }
  catch (e) { threw = e.message === 'malloc trap'; }
  if (!threw) throw Error('lost allocation exception');
}
print('PASS: exact helper gate; ASCII, empty, NUL, Unicode, surrogates, growth, malloc exceptions');
for (const input of cases) {
  reset(); growOnMalloc = true;
  const p = optimized(input, malloc, realloc);
  const expected = decode(mem.subarray(p, p + WASM_VECTOR_LEN));
  if (getStringFromWasm0(p, WASM_VECTOR_LEN) !== expected) throw Error('wasm decode mismatch');
}
for (const bytes of [[0x80], [0xc0,0xaf], [0xef,0xbb,0xbf,65], [0xf0,0x9f,0x98,0x80]]) {
  reset(); mem.set(bytes, 8);
  if (getStringFromWasm0(8, bytes.length) !== decode(mem.subarray(8, 8 + bytes.length))) throw Error('UTF-8/BOM behavior changed');
}
for (const [p,n] of [[mem.length,1], [0xffffffff,4], [1,0xffffffff]]) {
  let threw = false; try { getStringFromWasm0(p,n); } catch(e) { threw = e instanceof RangeError; }
  if (!threw) throw Error('missing wasm bounds check');
}
if (getStringFromWasm0(mem.length,0) !== '') throw Error('empty string at memory end');
print('PASS: direct decoding matches UTF-8/BOM behavior, follows moved memory and checks bounds');
for (const fn of [original, optimized]) {
  reset(); const start = Date.now();
  for (let i=0; i<200000; i++) { if (next > 8000000) next = 8; fn('texture-compression-bc', malloc, realloc); }
  print(fn.name + ': ' + (Date.now()-start) + ' ms / 200000 transfers');
}
'''
Path(sys.argv[1]).write_text(script, encoding='utf-8', newline='\n')
