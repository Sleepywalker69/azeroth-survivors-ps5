# Compile the JS side of the host (host.c, wasi.c, the generated bridge) and QuickJS into one
# static library for the Windows test host.
#
#   python -I build_host_lib.py --bridge <bridge_gen.c> --wasm2c-dir <wasm2c output>
#          --rt-dir <wabt>/wasm2c --quickjs-dir <quickjs-ng>/src --obj-dir <objects>
#          --out <aethus_host.lib> --vsenv <vsenv.txt> [--opt -O2]
#
# The wasm2c defines are build_wasm_lib's: host.c and the bridge reach the module's memory and
# tables through wasm-rt.h, whose structures and inline accessors depend on them. Objects are
# rebuilt when their source, or a header of ours or the translation's, is newer.
import argparse, pathlib, subprocess, sys, time

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
from build_wasm_lib import DEFINES, load_env  # noqa: E402

HOST_C = pathlib.Path(__file__).resolve().parent.parent / 'c'

# Our own files are compiled with warnings on; QuickJS is not ours to fix.
OWN_FLAGS = ['-std=c11', '-Wall', '-Wextra', '-Wno-unused-parameter', '-Wno-missing-field-initializers',
             '-D_CRT_SECURE_NO_WARNINGS', '-D_CRT_NONSTDC_NO_WARNINGS']
QUICKJS_FLAGS = ['-w', '-D_CRT_SECURE_NO_WARNINGS']


def stale(obj, inputs):
    if not obj.exists():
        return True
    t = obj.stat().st_mtime
    return any(p.stat().st_mtime > t for p in inputs)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--bridge', required=True)
    ap.add_argument('--wasm2c-dir', required=True)
    ap.add_argument('--rt-dir', required=True)
    ap.add_argument('--quickjs-dir', required=True)
    ap.add_argument('--obj-dir', required=True)
    ap.add_argument('--out', required=True)
    ap.add_argument('--vsenv', required=True)
    ap.add_argument('--opt', default='-O2')
    a = ap.parse_args()

    obj_dir = pathlib.Path(a.obj_dir)
    obj_dir.mkdir(parents=True, exist_ok=True)
    env = load_env(a.vsenv)
    wasm2c_dir, rt_dir, qjs_dir = pathlib.Path(a.wasm2c_dir), pathlib.Path(a.rt_dir), pathlib.Path(a.quickjs_dir)
    includes = [HOST_C, wasm2c_dir, rt_dir, qjs_dir]
    headers = [HOST_C / 'bridge.h', HOST_C / 'host.h', wasm2c_dir / 'wenilla.h', qjs_dir / 'quickjs.h']

    units = [
        (qjs_dir / 'quickjs-amalgam.c', 'quickjs.o', QUICKJS_FLAGS),
        (pathlib.Path(a.bridge), 'bridge_gen.o', OWN_FLAGS),
        (HOST_C / 'host.c', 'host.o', OWN_FLAGS),
        (HOST_C / 'wasi.c', 'wasi.o', OWN_FLAGS),
    ]
    t0 = time.time()
    failed = False
    for src, obj_name, flags in units:
        obj = obj_dir / obj_name
        if not stale(obj, [src, *headers]):
            continue
        cmd = ['clang', a.opt, '-g', '-c', *flags, *DEFINES, *[f'-I{i}' for i in includes], str(src), '-o', str(obj)]
        t = time.time()
        r = subprocess.run(cmd, env=env, capture_output=True, text=True)
        out = (r.stdout + r.stderr).strip()
        if out:
            print(out[-20000:], flush=True)
        if r.returncode != 0:
            print(f'FAILED {src.name}', flush=True)
            failed = True
            if obj.exists():
                obj.unlink()
        else:
            print(f'{src.name} {time.time() - t:.0f}s', flush=True)
    if failed:
        sys.exit('the host library was not built')
    objs = [str(obj_dir / name) for _, name, _ in units]
    r = subprocess.run(['llvm-lib', f'/out:{a.out}', *objs], env=env, capture_output=True, text=True)
    if r.returncode != 0:
        sys.exit(r.stderr or r.stdout)
    print(f'wrote {a.out} in {time.time() - t0:.0f}s')


if __name__ == '__main__':
    main()
