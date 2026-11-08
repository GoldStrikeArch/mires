#!/usr/bin/env python3
"""Check exact fresh-build comparator artifacts offline; do not equate this with browser parity."""
import argparse, datetime, hashlib, json, pathlib, subprocess
ROOT=pathlib.Path(__file__).resolve().parents[2]
p=argparse.ArgumentParser(description=__doc__);p.add_argument('--languages',default='rescript,melange,jsoo,scalajs,dart,kotlin,elm');p.add_argument('--suite',choices=('oss','pro','all'),default='all');a=p.parse_args()
out=ROOT/'build/comparators';(out/'logs').mkdir(parents=True,exist_ok=True)
rows=[]
for lang in a.languages.split(','):
 for suite in (('oss','pro') if a.suite=='all' else (a.suite,)):
  build=out/f'{suite}-{lang}.build.json'
  if not build.is_file() or not json.loads(build.read_text())['built']:
   rows.append({'language':lang,'suite':suite,'status':'not-built','passed':False});continue
  identity=json.loads(build.read_text())
  stale=[]
  for artifact in identity['artifacts']:
   file=ROOT/artifact['path']
   if not file.is_file() or hashlib.sha256(file.read_bytes()).hexdigest()!=artifact['sha256']:stale.append(artifact['path'])
  if stale:
   rows.append({'language':lang,'suite':suite,'status':'artifact-identity-mismatch','paths':stale,'passed':False});continue
  port=ROOT/('experiments' if suite=='oss' else 'experiments-pro')/(('fingerprintjs-' if suite=='oss' else 'fp-pro-')+lang)
  variants=[port/'dist']+[x for x in sorted((port/'dist').iterdir()) if x.is_dir() and (x/'fp.esm.min.js').is_file()]
  for dist in variants:
   variant='default' if dist==port/'dist' else dist.name;name=f'{suite}-{lang}-{variant}'
   checks=[('package',['node','experiments/tools/check-comparator-package.mjs',suite,str(dist.relative_to(ROOT)),f'build/comparators/{name}.package.json'])]
   if suite=='oss':checks.append(('kernels',['node','experiments/tools/check-oss-kernels.mjs',str((dist/'fp.esm.min.js').relative_to(ROOT)),f'build/comparators/{name}.kernels.json']))
   if suite=='oss':checks.append(('seeds',['node','experiments/tools/check-seed-boundaries.mjs',str((dist/'fp.esm.min.js').relative_to(ROOT)),f'build/comparators/{name}.seeds.json']))
   else:checks.append(('validation',['node','experiments-pro/shared/start-consumer.mjs','--candidate',str(dist/'fp.esm.min.js')]))
   for check,cmd in checks:
    path=out/'logs'/f'{name}-{check}.log'
    with path.open('w') as f:
     try:code=subprocess.run(cmd,cwd=ROOT,stdout=f,stderr=subprocess.STDOUT,timeout=60).returncode
     except subprocess.TimeoutExpired:code=124
    rows.append({'language':lang,'suite':suite,'variant':variant,'check':check,'exitCode':code,'passed':code==0,'log':str(path.relative_to(ROOT))})
    print(name,check,'PASS' if code==0 else 'FAIL',flush=True)
    if code:print('\n'.join(path.read_text(errors='replace').splitlines()[-6:]),flush=True)
result={'schema':1,'generatedAt':datetime.datetime.now(datetime.timezone.utc).isoformat(),'passed':all(x['passed'] for x in rows),'checks':rows,'scope':'offline artifacts only; not production, browser-parity, or timing qualification'}
path=out/('checks-'+a.suite+'-'+a.languages.replace(',','-')+'.json');path.write_text(json.dumps(result,indent=2)+'\n')
raise SystemExit(0 if result['passed'] else 1)
