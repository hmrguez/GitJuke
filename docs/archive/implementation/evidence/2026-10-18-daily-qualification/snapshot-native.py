"""Read only metadata from exactly the new synthetic qualification identity."""
import hashlib,json,pathlib,sqlite3,sys
stage=sys.argv[1];assert stage.replace('-','').isalnum()
p=pathlib.Path(__file__).resolve().parent
data=pathlib.Path.home()/'Library/Application Support/dev.hmrguez.gitjuke.dailyqualification'
db=data/'gitjuke.sqlite3';fixture=pathlib.Path('/private/tmp/gitjuke-daily-20261018/fixture')
with sqlite3.connect('file:'+str(db)+'?mode=ro',uri=True) as c:
 c.execute('pragma query_only=on')
 settings=json.loads(c.execute('select json from settings where id=1').fetchone()[0])
 r={'stage':stage,'schema':c.execute('pragma user_version').fetchone()[0],'integrity':c.execute('pragma quick_check').fetchone()[0],'paused':settings['paused'],'alerts':settings['alerts'],'seen':dict(c.execute('select * from seen')),'queries':list(c.execute('select id,name,search,version from saved_query')),'watchReasons':list(c.execute('select * from watch_source order by id,source')),'ledgerCounts':{t:c.execute('select count(*) from '+t).fetchone()[0] for t in ['seen','dedup','activity_intent','intent_member','alert_attempt']},'pendingLogs':c.execute("select count(*) from pending where job like '%Logs%'").fetchone()[0]}
 trace=[json.loads(line) for line in (fixture/'reads').read_text().splitlines()] if (fixture/'reads').exists() else []
 r['helperReads']=len(trace);r['logReads']=sum(any('/logs' in a for a in x['args']) for x in trace)
 canary=b'LOCAL DAILY LOG LINE 0';r['logCanaryPersisted']=any(canary in file.read_bytes() for file in data.iterdir() if file.is_file() and file.name.startswith('gitjuke.sqlite3'))
(p/(stage+'.json')).write_text(json.dumps(r,indent=2)+'\n');print(json.dumps({'stage':stage,'schema':r['schema'],'paused':r['paused'],'helperReads':r['helperReads'],'logReads':r['logReads'],'localSeen':{k:v for k,v in r['seen'].items() if ':local-' in k}},indent=2))
