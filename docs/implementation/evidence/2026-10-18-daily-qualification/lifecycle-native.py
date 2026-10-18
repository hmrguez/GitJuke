import pathlib,subprocess,json,sys,sqlite3
out=pathlib.Path(__file__).resolve().parent
exe=pathlib.Path('/Users/hmrguez/Programming/Personal/GitJuke/target/aarch64-apple-darwin/debug/bundle/macos/GitJuke Daily Qualification.app/Contents/MacOS/gitjuke')
data=pathlib.Path.home()/'Library/Application Support/dev.hmrguez.gitjuke.dailyqualification'
action,stage=sys.argv[1:];assert stage.replace('-','').isalnum()
def pids():
 s=subprocess.check_output(['ps','-axo','pid=,args='],text=True)
 return [int(p[0]) for l in s.splitlines() if len(p:=l.strip().split(None,1))==2 and (p[1]==str(exe) or p[1].startswith(str(exe)+' '))]
r={'beforePids':pids(),'action':action}
with (out/'native-process.log').open('ab') as log:
 if action=='race':
  children=[subprocess.Popen([str(exe)]+(['--background'] if n%2==0 else []),stdin=subprocess.DEVNULL,stdout=log,stderr=log) for n in range(8)]
  r['contenderExits']=[c.wait(timeout=8) for c in children]
 elif action=='background-off':
  assert not (data/'login.json').exists() or json.loads((data/'login.json').read_text())['enabled'] is False
  c=subprocess.Popen([str(exe),'--background'],stdin=subprocess.DEVNULL,stdout=log,stderr=log);r['backgroundExit']=c.wait(timeout=8)
 else:assert action=='snapshot'
r['afterPids']=pids();r['loginPreferenceExists']=(data/'login.json').exists();r['launchAgentExists']=(pathlib.Path.home()/'Library/LaunchAgents/dev.hmrguez.gitjuke.dailyqualification.plist').exists()
with sqlite3.connect('file:'+str(data/'gitjuke.sqlite3')+'?mode=ro',uri=True) as c:
 r['watchReasons']=list(c.execute('select * from watch_source order by id,source'));r['paused']=json.loads(c.execute('select json from settings where id=1').fetchone()[0])['paused'];r['selection']=c.execute("select value from progress where key='selection'").fetchone()
(out/(stage+'.json')).write_text(json.dumps(r,indent=2)+'\n');print(json.dumps(r,indent=2))
