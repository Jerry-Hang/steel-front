# -*- coding: utf-8 -*-
import json
from collections import Counter
rows = []
for l in open('data/llm_decisions.jsonl', encoding='utf-8'):
    try:
        rows.append(json.loads(l))
    except Exception as e:
        print('bad line:', str(e)[:60])
print('total', len(rows))
print('by side/accepted:', Counter((r.get('side'), r.get('accepted')) for r in rows))
print('by note head:', Counter(r.get('note','').split(':')[0] for r in rows))
acc = [r for r in rows if r.get('accepted')]
print('accepted examples:')
for r in acc[-4:]:
    print(' ', r['side'], r.get('decision','')[:120])