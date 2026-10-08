#!/usr/bin/env python3
"""Verify completeness and traceability of a V3.1 static document audit."""
import hashlib, json, re
from pathlib import Path
from collections import Counter

ROOT=Path(__file__).resolve().parent
SOURCE=ROOT.parent/'subtitle_tool_rust_planning_v3_1'
issues=json.loads((ROOT/'issues.json').read_text('utf-8'))
manifest=json.loads((ROOT/'BASELINE_SHA256.json').read_text('utf-8'))
wbs=json.loads((ROOT/'WBS_REBUILT.json').read_text('utf-8'))

assert SOURCE.is_dir(), f'Unresolved baseline: {SOURCE}'
assert len(manifest['files'])==42
assert len(issues)==38
assert len({i['id'] for i in issues})==len(issues)
assert [f'{n:02d}' for n in range(42)] == [x['name'][:2] for x in manifest['files']], 'Missing / duplicated document prefix'
source_lines={}
for item in manifest['files']:
    raw=(SOURCE/item['name']).read_bytes()
    assert hashlib.sha256(raw).hexdigest()==item['sha256'], f'Hash changed: {item["name"]}'
    lines=raw.decode('utf-8').splitlines()
    assert len(lines)==item['lines'], f'Line drift: {item["name"]}'
    source_lines[item['name'][:2]]=len(lines)

bad=[]
for item in issues:
    for match in item['refs'].split(';'):
        m=re.fullmatch(r'(\d\d):(\d+)(?:-(\d+))?', match)
        if not m:
            bad.append((item['id'],'invalid syntax',match)); continue
        prefix,start,end=m.group(1),int(m.group(2)),int(m.group(3) or m.group(2))
        if prefix not in source_lines or start<1 or end<start or end>source_lines.get(prefix,0):
            bad.append((item['id'],'out of range',match))
assert not bad, f'Invalid references: {bad}'
assert Counter(i['severity'] for i in issues)=={'P0':10,'P1':22,'P2':6}
ids={t['id'] for t in wbs}
assert len(wbs)==59 and len(ids)==len(wbs)
assert all(dep in ids for t in wbs for dep in t['deps'])
visited,visiting=set(),set()
tasks={t['id']:t for t in wbs}
def visit(t):
    if t in visited: return
    assert t not in visiting, f'WBS cycle around {t}'
    visiting.add(t)
    for parent in tasks[t]['deps']: visit(parent)
    visiting.remove(t);visited.add(t)
for task in tasks:visit(task)

reports=[p for p in ROOT.glob('*.md')]
assert len(reports)>=7
assert (ROOT/'03_IMPLEMENTATION_CONTRACTS.md').read_text('utf-8').find('相同2160p?') == -1
print('PASS: 42/42 baseline files exist, sizes+SHA256+line counts match')
print('PASS: all 38 issue IDs unique; P0=10, P1=22, P2=6')
print('PASS: all issue evidence references parse and are in source line bounds')
print('PASS: 59/59 WBS tasks unique, all dependencies defined, DAG acyclic')
print(f'PASS: {len(reports)} audit Markdown reports present')
