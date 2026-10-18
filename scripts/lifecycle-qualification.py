#!/usr/bin/env python3
"""Native process/registration observations for exactly the isolated lifecycle app.
Never creates a LaunchAgent or changes the normal app identity. Registration is tested
through the app's explicit controls. Run only after the qualification build/install.
"""
import argparse, json, os, pathlib, plistlib, sqlite3, subprocess, time
ID='dev.hmrguez.gitjuke.lifecycle-qualification'
BUNDLE=pathlib.Path('/private/tmp/GitJuke Lifecycle.app')
EXE=BUNDLE/'Contents/MacOS/gitjuke'
DATA=pathlib.Path.home()/'Library/Application Support'/ID
AGENT=pathlib.Path.home()/'Library/LaunchAgents'/f'{ID}.plist'
parser=argparse.ArgumentParser();parser.add_argument('action',choices=['snapshot','background','race','os-disable']);args=parser.parse_args()
source_bundle=BUNDLE if BUNDLE.exists() else pathlib.Path('/private/tmp/GitJuke Lifecycle.moved.app')
assert plistlib.loads((source_bundle/'Contents/Info.plist').read_bytes())['CFBundleIdentifier']==ID
if BUNDLE.exists():assert EXE.resolve()==EXE

def pids():
    result=subprocess.run(['/bin/ps','-axo','pid=,args='],capture_output=True,text=True,check=True)
    return [int(parts[0]) for line in result.stdout.splitlines() if len(parts:=line.strip().split(None,1))==2 and any(parts[1]==str(exe) or parts[1].startswith(str(exe)+' ') for exe in [EXE,pathlib.Path('/private/tmp/GitJuke Lifecycle.moved.app/Contents/MacOS/gitjuke')])]

if args.action=='os-disable':
    subprocess.run(['/bin/launchctl','disable',f'gui/{os.getuid()}/{ID}'],check=True)
elif args.action=='background':
    with open('/private/tmp/gitjuke-lifecycle-native-output.log','ab') as log:
        child=subprocess.Popen([str(EXE),'--background'],stdin=subprocess.DEVNULL,stdout=log,stderr=log)
    time.sleep(2);print(json.dumps({'launchedPid':child.pid,'exit':child.poll()}))
elif args.action=='race':
    before=pids()
    with open('/private/tmp/gitjuke-lifecycle-native-output.log','ab') as log:
        children=[subprocess.Popen([str(EXE)]+(['--background'] if n%2==0 else []),stdin=subprocess.DEVNULL,stdout=log,stderr=log) for n in range(8)]
    time.sleep(4)
    live=pids();assert len(live)==1,live
    assert all(c.poll() in [None,0] for c in children)
    print(json.dumps({'contenders':8,'beforePids':before,'afterPids':live,'contenderExitCodes':[c.poll() for c in children]}))
    if before:assert live==before

preference=json.loads((DATA/'login.json').read_text()) if (DATA/'login.json').exists() else {'enabled':False}
agent=plistlib.loads(AGENT.read_bytes()) if AGENT.is_file() else None
if agent:
    assert agent['Label']==ID
    assert agent['ProgramArguments']==[str(EXE),'--background']
    assert agent['RunAtLoad'] is True
    assert not any(key in agent for key in ['KeepAlive','StartInterval','StartCalendarInterval','WatchPaths','QueueDirectories'])
observations={'identity':ID,'pids':pids(),'preference':preference,'registeredPathExists':AGENT.exists(),'registrationFileValid':agent is not None,'stablePath':str(EXE),'stablePathExists':EXE.exists(),'noWatchdog':agent is None or 'KeepAlive' not in agent}
if AGENT.is_file():
    import hashlib
    observations['registrationSha256']=hashlib.sha256(AGENT.read_bytes()).hexdigest()
    observations['registrationModifiedNs']=AGENT.stat().st_mtime_ns
os_status=subprocess.run(['/bin/launchctl','print-disabled',f'gui/{os.getuid()}'],capture_output=True,text=True,check=True).stdout
matches=[line.split('=>',1)[1].strip() for line in os_status.splitlines() if line.strip().startswith(f'"{ID}" =>')]
observations['osDisabled']=bool(matches and matches[0] in ['true','disabled'])

if (DATA/'gitjuke.sqlite3').exists():
    with sqlite3.connect(f'file:{DATA}/gitjuke.sqlite3?mode=ro',uri=True) as db:
        settings=json.loads(db.execute('select json from settings where id=1').fetchone()[0]);observations['paused']=settings['paused']
        observations['integrity']=db.execute('pragma integrity_check').fetchone()[0]
        observations['schema']=db.execute('pragma user_version').fetchone()[0]
        observations['ledgerCounts']={table:db.execute(f'select count(*) from {table}').fetchone()[0] for table in ['seen','dedup','activity_intent','alert_attempt']}
print(json.dumps(observations,indent=2))
