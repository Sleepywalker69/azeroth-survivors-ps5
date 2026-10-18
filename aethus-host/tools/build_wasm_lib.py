# Compile the wasm2c translation of the web build (plus wasm2c's runtime) into one static
# library for the Windows test host.
#
#   python -I build_wasm_lib.py --c-dir <wasm2c output> --rt-dir <wabt>/wasm2c
#          --obj-dir <objects> --out <wenilla.lib> --vsenv <vsenv.txt> [--jobs 8] [--only N]
#
# vsenv.txt holds `set` output captured after vcvars64.bat, so clang finds the MSVC and
# Windows SDK headers. Objects are rebuilt only when their source is newer.
import argparse, concurrent.futures, os, pathlib, subprocess, sys, time

# The same memory and stack model as the console build: the linear memory is reserved up front
# and committed as it grows (so it never moves, and functions may keep its base in a local),
# accesses are bounds-checked (no guard pages or signal handlers), and there is no call-depth
# counting (the host runs the module on a large stack). The host's own C files that include
# wasm-rt.h must be compiled with the same defines.
DEFINES = [
    '-DWASM_RT_USE_MMAP=1',
    '-DWASM_RT_MEMCHECK_BOUNDS_CHECK=1',
    '-DWASM_RT_NONCONFORMING_UNCHECKED_STACK_EXHAUSTION=1',
]


def load_env(path):
    env = dict(os.environ)
    for line in pathlib.Path(path).read_text(encoding='utf-8-sig', errors='replace').splitlines():
        key, sep, value = line.partition('=')
        if sep and key.upper() in ('INCLUDE', 'LIB', 'LIBPATH'):
            env[key.upper()] = value
    return env


def compile_one(src, obj, includes, env, opt):
    if obj.exists() and obj.stat().st_mtime >= src.stat().st_mtime:
        return src.name, 0.0, ''
    t0 = time.time()
    cmd = ['clang', opt, '-c', '-w', '-fno-strict-aliasing', *DEFINES]
    cmd += [f'-I{i}' for i in includes]
    cmd += [str(src), '-o', str(obj)]
    r = subprocess.run(cmd, env=env, capture_output=True, text=True)
    if r.returncode != 0:
        return src.name, -1.0, r.stderr[-4000:]
    return src.name, time.time() - t0, ''


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--c-dir', required=True)
    ap.add_argument('--rt-dir', required=True)
    ap.add_argument('--obj-dir', required=True)
    ap.add_argument('--out', required=True)
    ap.add_argument('--vsenv', required=True)
    ap.add_argument('--jobs', type=int, default=8)
    ap.add_argument('--opt', default='-O2')
    ap.add_argument('--only', type=int, default=None, help='compile one translated file and stop')
    a = ap.parse_args()

    c_dir, rt_dir, obj_dir = pathlib.Path(a.c_dir), pathlib.Path(a.rt_dir), pathlib.Path(a.obj_dir)
    obj_dir.mkdir(parents=True, exist_ok=True)
    env = load_env(a.vsenv)
    includes = [rt_dir, c_dir]

    sources = sorted(c_dir.glob('wenilla_*.c'), key=lambda p: int(p.stem.split('_')[1]))
    if a.only is not None:
        sources = [c_dir / f'wenilla_{a.only}.c']
    else:
        sources += [rt_dir / 'wasm-rt-impl.c', rt_dir / 'wasm-rt-mem-impl.c', rt_dir / 'wasm-rt-exceptions-impl.c']
    # Largest first, so the long poles start early.
    sources.sort(key=lambda p: -p.stat().st_size)

    t0 = time.time()
    failed = []
    with concurrent.futures.ThreadPoolExecutor(a.jobs) as pool:
        futs = [pool.submit(compile_one, s, obj_dir / (s.stem + '.o'), includes, env, a.opt) for s in sources]
        for done, f in enumerate(concurrent.futures.as_completed(futs), 1):
            name, secs, err = f.result()
            if secs < 0:
                failed.append(name)
                print(f'FAILED {name}\n{err}', flush=True)
            elif secs > 0:
                print(f'[{done}/{len(futs)}] {name} {secs:.0f}s (total {time.time() - t0:.0f}s)', flush=True)
    if failed:
        sys.exit(f'{len(failed)} files failed')
    if a.only is not None:
        return
    objs = [str(obj_dir / (s.stem + '.o')) for s in sources]
    rsp = obj_dir / 'objs.rsp'
    rsp.write_text('\n'.join(f'"{o}"' for o in objs))
    r = subprocess.run(['llvm-lib', f'/out:{a.out}', f'@{rsp}'], env=env, capture_output=True, text=True)
    if r.returncode != 0:
        sys.exit(r.stderr or r.stdout)
    print(f'wrote {a.out} in {time.time() - t0:.0f}s')


if __name__ == '__main__':
    main()
