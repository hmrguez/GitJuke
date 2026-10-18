import json,pathlib,sqlite3,shutil,subprocess
repo=pathlib.Path('/Users/hmrguez/Programming/Personal/GitJuke')
base=pathlib.Path('/private/tmp/gitjuke-daily-20261018')
fixture=base/'fixture'
seed=base/'dev.hmrguez.gitjuke.qualification.accessibility'
data=pathlib.Path.home()/'Library/Application Support/dev.hmrguez.gitjuke.dailyqualification'
assert not base.exists() and not data.exists(), 'Refuse existing qualification inputs'
base.mkdir()
subprocess.run(['cargo','run','--offline','-p','gitjuke-core','--example','reader_qualification','--',str(fixture),str(seed)],cwd=repo,check=True)
s=json.loads((fixture/'scenario.json').read_text());s.update(query_ids=list(range(1,151)),actions_logs=True,log_text='\n'.join('LOCAL DAILY LOG LINE '+str(i) for i in range(402)))
(fixture/'scenario.json').write_text(json.dumps(s))
with sqlite3.connect(seed/'gitjuke.sqlite3') as c:
 settings=json.loads(c.execute('select json from settings where id=1').fetchone()[0]);settings['alerts']['enabled']=False;settings['alerts']['onboarded']=False
 c.execute('update settings set json=? where id=1',(json.dumps(settings),))
 assert c.execute('pragma user_version').fetchone()[0]==16
shutil.copytree(seed,data)
print(json.dumps({'fixture':str(fixture),'database':str(data/'gitjuke.sqlite3'),'schema':16,'paused':True,'remoteAccess':False,'alertsEnabled':False}))
