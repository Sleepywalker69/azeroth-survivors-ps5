#!/usr/bin/env python3
"""Use native UTF-8 operations for reviewed wasm-bindgen string helpers.

QuickJS interprets wasm-bindgen's ASCII byte loop. Its existing TextEncoder path
does the same transfer using native conversion and typed-array copies. String reads
use the host's bounds-checked direct decoder. Only exact known helpers are changed;
unfamiliar glue stops for review. Source downloads
remain untouched. Run with Python -I.
"""
import argparse
import hashlib
from pathlib import Path

ORIGINAL = """function passStringToWasm0(arg, malloc, realloc) {
    if (realloc === undefined) {
        const buf = cachedTextEncoder.encode(arg);
        const ptr = malloc(buf.length, 1) >>> 0;
        getUint8ArrayMemory0().subarray(ptr, ptr + buf.length).set(buf);
        WASM_VECTOR_LEN = buf.length;
        return ptr;
    }

    let len = arg.length;
    let ptr = malloc(len, 1) >>> 0;

    const mem = getUint8ArrayMemory0();

    let offset = 0;

    for (; offset < len; offset++) {
        const code = arg.charCodeAt(offset);
        if (code > 0x7F) break;
        mem[ptr + offset] = code;
    }
    if (offset !== len) {
        if (offset !== 0) {
            arg = arg.slice(offset);
        }
        ptr = realloc(ptr, len, len = offset + arg.length * 3, 1) >>> 0;
        const view = getUint8ArrayMemory0().subarray(ptr + offset, ptr + len);
        const ret = cachedTextEncoder.encodeInto(arg, view);

        offset += ret.written;
        ptr = realloc(ptr, len, offset, 1) >>> 0;
    }

    WASM_VECTOR_LEN = offset;
    return ptr;
}"""

OPTIMIZED = """function passStringToWasm0(arg, malloc, realloc) {
    // Aethus host: native UTF-8 conversion avoids an interpreted loop per byte.
    const buf = cachedTextEncoder.encode(arg);
    const ptr = malloc(buf.length, 1) >>> 0;
    getUint8ArrayMemory0().subarray(ptr, ptr + buf.length).set(buf);
    WASM_VECTOR_LEN = buf.length;
    return ptr;
}"""

ORIGINAL_GET = """function getStringFromWasm0(ptr, len) {
    return decodeText(ptr >>> 0, len);
}"""

ORIGINAL_DECODE = """function decodeText(ptr, len) {
    numBytesDecoded += len;
    if (numBytesDecoded >= MAX_SAFARI_DECODE_BYTES) {
        cachedTextDecoder = new TextDecoder('utf-8', { ignoreBOM: true, fatal: true });
        cachedTextDecoder.decode();
        numBytesDecoded = len;
    }
    return cachedTextDecoder.decode(getUint8ArrayMemory0().subarray(ptr, ptr + len));
}"""

OPTIMIZED_GET = """function getStringFromWasm0(ptr, len) {
    return globalThis.__native.textDecodeWasm(ptr >>> 0, len);
}"""


def optimize(source):
    if source.count(ORIGINAL) != 1 or source.count('function passStringToWasm0(') != 1:
        raise ValueError('unreviewed passStringToWasm0 helper; refusing to patch glue')
    if (source.count(ORIGINAL_GET) != 1 or source.count(ORIGINAL_DECODE) != 1 or
            source.count('function getStringFromWasm0(') != 1 or source.count('function decodeText(') != 1):
        raise ValueError('unreviewed getStringFromWasm0/decodeText helper; refusing to patch glue')
    return source.replace(ORIGINAL, OPTIMIZED, 1).replace(ORIGINAL_GET, OPTIMIZED_GET, 1)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('source', type=Path)
    parser.add_argument('output', type=Path)
    args = parser.parse_args()
    if args.source.resolve() == args.output.resolve():
        parser.error('source download and output must be different paths')
    source = args.source.read_text(encoding='utf-8')
    patched = optimize(source)
    args.output.write_text(patched, encoding='utf-8', newline='\n')
    print('glue: native UTF-8 helpers; output sha256 ' +
          hashlib.sha256(patched.encode('utf-8')).hexdigest())


if __name__ == '__main__':
    main()
