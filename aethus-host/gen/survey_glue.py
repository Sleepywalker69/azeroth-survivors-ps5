# Survey the parsed glue: which global names and helper functions the import bodies use,
# and which member names (properties and methods) they touch.
import json, re, sys, collections

recs = json.load(open(sys.argv[1], encoding='utf-8'))
KEYWORDS = {'const', 'let', 'var', 'return', 'new', 'typeof', 'true', 'false', 'null', 'undefined',
            'try', 'finally', 'if', 'else', 'instanceof', 'in', 'of', 'for', 'function', 'this',
            'throw', 'catch'}
STR = re.compile(r'"(?:[^"\x5c]|\x5c.)*"|\x27(?:[^\x27\x5c]|\x5c.)*\x27')
ids = collections.Counter()
members = collections.Counter()
for r in recs:
    b = STR.sub('""', r['body'])
    locals_ = set(re.findall(r'(?:const|let|var)\s+([A-Za-z_$][\w$]*)', b))
    for m in re.finditer(r'(?<![.\w$])([A-Za-z_$][\w$]*)', b):
        w = m.group(1)
        if re.match(r'arg\d+$', w) or w in KEYWORDS or w in locals_:
            continue
        ids[w] += 1
    for m in re.finditer(r'\.([A-Za-z_$][\w$]*)', b):
        members[m.group(1)] += 1
print('GLOBALS', sorted(ids.items(), key=lambda x: -x[1]))
print()
print('MEMBERS', len(members))
print(' '.join(sorted(members)))
