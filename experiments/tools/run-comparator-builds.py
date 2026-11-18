#!/usr/bin/env python3
"""Build restored comparator sources; retain exit codes instead of treating old dist files as passes."""
import argparse, datetime, hashlib, json, os, pathlib, signal, subprocess, time
ROOT = pathlib.Path(__file__).resolve().parents[2]
LANGUAGES = ('rescript', 'melange', 'jsoo', 'scalajs', 'dart', 'kotlin', 'elm', 'gopherjs')

def source_inventory(port, language, suite):
    """Hash authored inputs, excluding generated code and compiler caches."""
    excluded = {'node_modules','dist','_build','build','target','elm-stuff','kotlin-emit'}
    if language == 'rescript': excluded.add('lib')
    generated = {'compiled.js','elm.js','elm.raw.js','.static-agent-entry.mjs'}
    extensions = {'.res','.resi','.ml','.mli','.elm','.scala','.kt','.dart','.js','.mjs','.cjs','.json','.sbt','.kts','.toml','.yaml','.yml','.go'}
    files = {}
    for file in sorted(port.rglob('*')):
        if not file.is_file(): continue
        parts = file.relative_to(port).parts
        if any(x in excluded or x.startswith('.') for x in parts[:-1]): continue
        if file.name in generated or file.name.endswith('.res.mjs'): continue
        if language == 'rescript' and suite == 'pro' and parts == ('src','index.js'): continue
        if file.suffix not in extensions and file.name not in {'dune','dune-project','pubspec.lock','go.mod','go.sum'}: continue
        files[str(file.relative_to(ROOT))] = hashlib.sha256(file.read_bytes()).hexdigest()
    for name in ['package.json','package-lock.json','experiments-pro/shared/package-port.mjs','experiments/tools/kotlin']:
        file=ROOT/name
        if file.is_file():files[name]=hashlib.sha256(file.read_bytes()).hexdigest()
    return files

p = argparse.ArgumentParser(description=__doc__)
p.add_argument('--languages', default=','.join(LANGUAGES))
p.add_argument('--suite', choices=('oss','pro','all'), default='all')
p.add_argument('--timeout', type=int, default=1200)
p.add_argument('--output', default='build/comparators')
a = p.parse_args()
languages = a.languages.split(',')
if not languages or any(x not in LANGUAGES for x in languages):p.error('unknown language')
if a.timeout < 1:p.error('timeout must be positive')
out=ROOT/a.output; (out/'logs').mkdir(parents=True,exist_ok=True)
env=dict(os.environ)
env['PATH']=str(ROOT/'node_modules/.bin')+os.pathsep+str(pathlib.Path.home()/'.local/bin')+os.pathsep+env.get('PATH','')
env.setdefault('MIRES_OPAM_ROOT',str(ROOT/'.toolchains/opam'))
env.setdefault('MIRES_OPAM_SWITCH','mires-comparators')
env.setdefault('KOTLIN_BIN',str(ROOT/'experiments/tools/kotlin'))
env.setdefault('DART','dart')
env.setdefault('DUNE_JOBS','4')
env.setdefault('MIRES_GO_ROOT',str(ROOT/'.toolchains/go'))
env.setdefault('COURSIER_REPOSITORIES','https://repo.maven.apache.org/maven2')
suites=('oss','pro') if a.suite=='all' else (a.suite,)
failed=False
for lang in languages:
 for suite in suites:
  name=f'{suite}-{lang}'; port=ROOT/('experiments' if suite=='oss' else 'experiments-pro')/(('fingerprintjs-' if suite=='oss' else 'fp-pro-')+lang)
  log=out/'logs'/(name+'.log'); receipt=out/(name+'.build.json')
  if log.exists() or receipt.exists():
   import shutil
   previous=out/'attempts'/name/str(time.time_ns());previous.mkdir(parents=True)
   for old in (log,receipt):
    if old.exists():shutil.copy2(old,previous/old.name)
  started=datetime.datetime.now(datetime.timezone.utc).isoformat(); start=time.monotonic()
  command=['node','tools/build.mjs']; timed=False
  source_before=source_inventory(port,lang,suite)
  # Old output remains identifiable but cannot be mistaken for a successful fresh build.
  if (port/'dist').exists():
   previous=port/('build/previous-dist-'+str(time.time_ns())); previous.parent.mkdir(parents=True,exist_ok=True); (port/'dist').rename(previous)
  print('BUILD',name,flush=True)
  with log.open('w') as f:
   process=subprocess.Popen(command,cwd=port,env=env,stdout=f,stderr=subprocess.STDOUT,start_new_session=True)
   try: code=process.wait(timeout=a.timeout)
   except subprocess.TimeoutExpired:
    timed=True;os.killpg(process.pid,signal.SIGTERM)
    try:process.wait(timeout=10)
    except subprocess.TimeoutExpired:os.killpg(process.pid,signal.SIGKILL);process.wait()
    code=124
  files=[]
  for file in sorted((port/'dist').rglob('*')):
   if file.is_file():data=file.read_bytes();files.append({'path':str(file.relative_to(ROOT)),'bytes':len(data),'sha256':hashlib.sha256(data).hexdigest()})
  source_after=source_inventory(port,lang,suite)
  result={'schema':2,'sourceBefore':source_before,'sourceAfter':source_after,'sourceUnchanged':source_before==source_after,'startedAt':started,'finishedAt':datetime.datetime.now(datetime.timezone.utc).isoformat(),'language':lang,'suite':suite,'command':command,'cwd':str(port.relative_to(ROOT)),'exitCode':code,'timedOut':timed,'seconds':round(time.monotonic()-start,3),'log':str(log.relative_to(ROOT)),'built':code==0 and bool(files) and source_before==source_after,'artifacts':files,'qualification':'build only; no behavior or performance qualification'}
  receipt.write_text(json.dumps(result,indent=2)+'\n'); failed |= not result['built']
  print(name,'PASS' if result['built'] else 'FAIL',code,'seconds',result['seconds'],flush=True)
  if code:print('\n'.join(log.read_text(errors='replace').splitlines()[-14:]),flush=True)
raise SystemExit(1 if failed else 0)
