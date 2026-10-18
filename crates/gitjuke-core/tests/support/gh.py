#!/usr/bin/python3
"""Deterministic read-only gh subprocess fixture; never used by production."""
import json,sys,pathlib,urllib.parse,time,calendar,hashlib
root=pathlib.Path(sys.argv[0]).parent
config=json.loads((root/'scenario.json').read_text()) if (root/'scenario.json').exists() else {}
mode=(root/'mode').read_text() if (root/'mode').exists() else ''
args=sys.argv[1:]
query=json.loads(sys.stdin.read() or '{}') if 'graphql' in args else {}
with (root/'reads').open('a') as f:f.write(json.dumps({'args':args,'query':query.get('query','').split('(')[0],'variables':query.get('variables',{})})+'\n')
if args==['--version']:print('gh version test-fixture');sys.exit(0)
if args[0]=='auth':print(json.dumps({'hosts':{'github.com':[{'state':'success','active':True,'login':'reader','tokenSource':'keyring'}]}}));sys.exit(0)
identity={'databaseId':99 if mode=='mismatch' else 42,'login':'reader'}
def pr(n):return {'id':'PR_'+str(n),'number':n,'title':'Change '+str(n),'url':'https://github.com/a/b/pull/'+str(n),'state':'CLOSED' if config.get('closed')==n else 'OPEN','isDraft':False,'updatedAt':'2026-10-17T00:00:00Z','author':{'login':'reader'},'repository':{'databaseId':1,'nameWithOwner':'a/b'}}
def issue(n):
 p=pr(n);return {'node_id':p['id'],'number':n,'title':p['title'],'html_url':p['url'],'state':p['state'].lower(),'draft':False,'updated_at':p['updatedAt'],'user':{'login':'reader'},'repository_url':'https://api.github.com/repos/a/b'}
def commit(sha,offset=0):
 count=config.get('contexts',2);size=int(query.get('variables',{}).get('first',25));nodes=[]
 for i in range(offset,min(count,offset+size)):
  if i%2:nodes.append({'__typename':'StatusContext','id':'SC_'+str(i),'context':'External '+str(i),'state':'SUCCESS','createdAt':config.get('check_time','2026-10-17T00:00:00Z'),'targetUrl':'https://ci.example/check'})
  else:nodes.append({'__typename':'CheckRun','id':'CR_'+str(i),'name':'Actions '+str(i),'status':'COMPLETED','conclusion':'SUCCESS','detailsUrl':'https://github.com/a/b/actions/runs/2','checkSuite':{'id':'CS_'+str(i),'status':'COMPLETED','conclusion':'SUCCESS','updatedAt':config.get('check_time','2026-10-17T00:00:00Z'),'commit':{'oid':sha},'workflowRun':{'databaseId':2+i,'runAttempt':config.get('attempt',1)}}})
 return {'oid':sha,'statusCheckRollup':{'contexts':{'totalCount':count,'pageInfo':{'hasNextPage':offset+size<count,'endCursor':str(offset+size) if offset+size<count else None},'nodes':nodes}}}
status=200;remaining=4900;bucket='core'
if query and config.get('overflow_batches') and query['query'].startswith('query WatchHeads') and len(query.get('variables',{}).get('ids',[]))>2:
 sys.stdout.write('x'*(16*1024*1024+1));sys.exit(0)
if query:
 bucket='graphql';q=query['query'];v=query.get('variables',{})
 if q.startswith('query Authored'):
  total=config.get('authored',1);offset=int(v.get('after') or 0);size=int(v.get('first',100));ids=config.get('authored_ids',list(range(1,total+1)))
  body={'data':{'viewer':dict(identity,pullRequests={'totalCount':len(ids),'pageInfo':{'hasNextPage':offset+size<len(ids),'endCursor':str(offset+size) if offset+size<len(ids) else None},'nodes':[pr(n) for n in ids[offset:offset+size]]})}}
 elif q.startswith('query WatchHeads'):
  nodes=[]
  for ident in v['ids']:
   n=int(ident.split('_')[-1]);p=pr(n);sha=config.get('head','abc');p.update(headRefOid=sha,reviewDecision='APPROVED',potentialMergeCommit=commit('merge-'+sha) if config.get('merge') else None,commits={'nodes':[{'commit':commit(sha)}]});nodes.append(p)
  if config.get('missing_head'):nodes=[None if ident=='PR_'+str(config['missing_head']) else node for ident,node in zip(v['ids'],nodes)]
  body={'data':{'viewer':identity,'nodes':nodes}}
  if config.get('deny_checks'):body['errors']=[{'type':'FORBIDDEN','path':['nodes',i,'commits']} for i in range(len(nodes))]
 elif q.startswith('query CheckContexts'):
  if config.get('slow_page'):time.sleep(config['slow_page'])
  body={'data':{'viewer':identity,'repository':{'object':commit(v['oid'],int(v['after']))}}}
  if config.get('deny_contexts'):body={'data':{'viewer':identity,'repository':None},'errors':[{'type':'FORBIDDEN','path':['repository','object','statusCheckRollup']}]}
  if config.get('deny_contexts_http'):status=403;body={'message':'Forbidden'}
 else:body={'errors':[{'message':'unknown test operation'}]}
 if 'data' in body:body['data']['rateLimit']={'cost':1,'remaining':remaining,'resetAt':'2099-01-01T00:00:00Z'}
else:
 endpoint=next((a for a in args if a=='user' or a.startswith(('repos/','user/','notifications?','search/'))),args[-1])
 if endpoint=='user':body={'id':identity['databaseId'],'login':'reader'}
 elif endpoint.startswith('user/teams'):
  body=[] if config.get('no_teams') else [{'node_id':'T_1','slug':'reviewers','organization':{'login':'o'}}]
 elif endpoint.startswith('search/issues'):
  bucket='search';url=urllib.parse.urlparse(endpoint);params=urllib.parse.parse_qs(url.query);q=params['q'][0];page=int(params.get('page',['1'])[0]);size=int(params.get('per_page',['100'])[0]);ids=config.get('team_ids',[1,3]) if 'team-review-requested:' in q else config.get('direct_ids',list(range(1,config['direct_count']+1)) if 'direct_count' in config else [1,2]);
  if 'created:' in q:
   interval=q.split('created:')[1].split()[0];lo,hi=interval.split('..');lo=calendar.timegm(time.strptime(lo,'%Y-%m-%dT%H:%M:%SZ'));hi=calendar.timegm(time.strptime(hi,'%Y-%m-%dT%H:%M:%SZ'));ids=[n for n in ids if lo<=1700000000+n<=hi]
  if config.get('deny_team') and 'team-review-requested:' in q:status=403;body={'message':'Forbidden'}
  else:
   body={'total_count':len(ids),'incomplete_results':bool(config.get('incomplete')),'items':[issue(n) for n in ids[(page-1)*size:page*size]]}
 elif '/stacks?' in endpoint:body=[]
 elif endpoint.startswith('notifications?'):
  entries=config.get('notifications',[]);params=urllib.parse.parse_qs(urllib.parse.urlparse(endpoint).query);page=int(params.get('page',['1'])[0]);size=int(params.get('per_page',['50'])[0]);body=entries[(page-1)*size:page*size]
 elif any(part in endpoint for part in ['/comments?','/reviews?','/timeline?']):
  source='timeline' if '/timeline?' in endpoint else 'reviews' if '/reviews?' in endpoint else 'review_comments' if '/pulls/' in endpoint else 'comments'
  if config.get('deny_activity')==source:status=403;body={'message':'Forbidden'}
  else:
   entries=config.get(source,[]);params=urllib.parse.parse_qs(urllib.parse.urlparse(endpoint).query);page=int(params.get('page',['1'])[0]);size=int(params.get('per_page',['50'])[0]);body=entries[(page-1)*size:page*size]
 elif endpoint.startswith('repos/a/b/pulls/'):
  n=int(endpoint.split('/')[-1]);p=pr(n);body={'node_id':p['id'],'number':n,'title':p['title'],'html_url':p['url'],'body':'<script>untrusted</script>','body_html':'&lt;script&gt;untrusted&lt;/script&gt;','created_at':'2026-10-17T00:00:00Z','base':{'ref':'main','repo':{'full_name':'a/b','id':1}},'head':{'sha':config.get('head','abc')},'state':p['state'].lower(),'draft':False,'user':{'login':'reader','id':42}}
 else:status=404;body={'message':'Not Found'}
if not query and config.get('deny_overview') and endpoint=='repos/a/b/pulls/'+str(config['deny_overview']):status=404;body={'message':'Not Found'}
if config.get('limit_bucket')==bucket:status=429;body={'message':'secondary rate limit'}
extra=''
if config.get('validators') and not query and endpoint.startswith('repos/'):
 etag='"'+hashlib.sha256(json.dumps(body,sort_keys=True).encode()).hexdigest()+'"'
 extra+='ETag: '+etag+'\r\n'
 if 'If-None-Match: '+etag in args:status=304;body=None

if not query and endpoint.startswith('notifications?') and len(entries)>page*size:extra+='Link: <https://api.github.com/next>; rel="next"\r\n'
if endpoint.startswith('notifications?') if not query else False:extra+='X-Poll-Interval: '+str(config.get('poll_interval',60))+'\r\n'
if not query and not config.get('deny_activity') and any(part in endpoint for part in ['/comments?','/reviews?','/timeline?']) and len(entries)>page*size:extra+='Link: <https://api.github.com/next>; rel="next"\r\n'
sys.stdout.write('HTTP/2.0 '
+str(status)+' Status\r\nX-RateLimit-Resource: '+bucket+'\r\nX-RateLimit-Remaining: '+str(remaining)+'\r\nX-RateLimit-Reset: 4070908800\r\n'+extra+('Retry-After: 60\r\n' if status==429 else '')+'\r\n'+(json.dumps(body) if status!=304 else ''))
if status>=400:sys.exit(1)
