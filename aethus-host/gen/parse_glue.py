# Parse wasm-bindgen's generated glue (wenilla.js) into one record per import:
# name, parameter list, whether it is wrapped in handleError, and the body text.
import json, re, sys
from pathlib import Path

def parse(path):
    src = Path(path).read_text(encoding='utf-8')
    start = src.index('function __wbg_get_imports()')
    end = src.index('\n    };\n    return {', start)
    body = src[start:end]
    out = []
    pat = re.compile(r'\n        (__wbg_[A-Za-z0-9_]+|__wbindgen_[A-Za-z0-9_]+): function\(([^)]*)\) \{')
    ms = list(pat.finditer(body))
    for i, m in enumerate(ms):
        stop = ms[i + 1].start() if i + 1 < len(ms) else len(body)
        text = body[m.end():stop].rstrip().rstrip(',').rstrip()
        assert text.endswith('}'), m.group(1)
        text = text[:-1].strip()
        name, params = m.group(1), [p.strip() for p in m.group(2).split(',') if p.strip()]
        handle = False
        hm = re.match(r'return handleError\(function \(([^)]*)\) \{(.*)\}, arguments\);?\s*$', text, re.S)
        if hm:
            handle = True
            params = [p.strip() for p in hm.group(1).split(',') if p.strip()]
            text = hm.group(2).strip()
        out.append({'name': name, 'params': params, 'handle_error': handle, 'body': text})
    return out

if __name__ == '__main__':
    recs = parse(sys.argv[1])
    json.dump(recs, open(sys.argv[2], 'w', encoding='utf-8'), indent=1)
    print(len(recs), 'imports;', sum(r['handle_error'] for r in recs), 'wrapped in handleError')
