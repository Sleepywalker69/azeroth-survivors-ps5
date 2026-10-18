# Print the glue imports whose bodies are more than one simple statement, to see which JS
# constructs the generator has to handle.
import json, re, sys

recs = json.load(open(sys.argv[1], encoding='utf-8'))
seen = set()
for r in recs:
    b = r['body']
    norm = re.sub(r'\s+', ' ', re.sub(r'\barg\d+\b', 'A', re.sub(r'\.\w+', '.P', b)))
    stmts = b.count(';')
    if stmts <= 2 and '{' not in b and 'Cast intrinsic' not in b:
        continue
    if norm in seen:
        continue
    seen.add(norm)
    print('==', r['name'], r['params'], 'handleError' if r['handle_error'] else '')
    print(b)
