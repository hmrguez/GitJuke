#!/usr/bin/python3
"""Deterministic gh subprocess fixture; never used by production."""
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
  else:nodes.append({'__typename':'CheckRun','id':'CR_'+str(i),'name':'Actions '+str(i),'status':'COMPLETED','conclusion':'SUCCESS','detailsUrl':('https://github.com/a/b/actions/runs/'+str(2+i)+'/job/'+str(100+i)) if config.get('actions_logs') else 'https://github.com/a/b/actions/runs/2','checkSuite':{'id':'CS_'+str(i),'status':'COMPLETED','conclusion':'SUCCESS','updatedAt':config.get('check_time','2026-10-17T00:00:00Z'),'commit':{'oid':sha},'workflowRun':{'databaseId':2+i,'runAttempt':config.get('attempt',1)}}})
 return {'oid':sha,'statusCheckRollup':{'contexts':{'totalCount':count,'pageInfo':{'hasNextPage':offset+size<count,'endCursor':str(offset+size) if offset+size<count else None},'nodes':nodes}}}
status=200;remaining=4900;bucket='core'
if query and config.get('overflow_batches') and query['query'].startswith('query WatchHeads') and len(query.get('variables',{}).get('ids',[]))>2:
 sys.stdout.write('x'*(16*1024*1024+1));sys.exit(0)
if query:
 bucket='graphql';q=query['query'];v=query.get('variables',{})
 if q.startswith('mutation'):
  node={'id':'posted','url':'https://github.com/a/b/pull/1#issuecomment-999','body':v['body'],'createdAt':'2026-10-19T10:00:00Z','author':{'login':'reader'}}
  body={'data':{'addPullRequestReviewThreadReply':{'comment':node}}} if 'addPullRequestReviewThreadReply' in q else {'data':{'addComment':{'commentEdge':{'node':node}}}}
  if config.get('deny_comment'):body={'errors':[{'type':'FORBIDDEN'}]}
 elif q.startswith('query AlertHead'):
  body={'data':{'viewer':identity,'repository':{'pullRequest':{'headRefOid':config.get('alert_head',config.get('head','abc')),'potentialMergeCommit':{'oid':'merge-abc'} if config.get('merge') else None}}}}
 elif q.startswith('query ConversationDestination'):
  if config.get('slow_conversation'):
   (root/'conversation-started').write_text('started');time.sleep(config['slow_conversation'])
  node={'__typename':config.get('destination_type','IssueComment'),'id':config.get('destination_id',v['thread']),'body':config.get('conversation_body','hello destination'),'createdAt':'2026-10-18T00:00:00Z','url':'https://github.com/a/b/pull/1#issuecomment-9','state':'APPROVED','fullDatabaseId':'9'}
  fixture=config.get('destinations',{}).get(v['thread'],{})
  node.update(fixture.get('node',{}))
  owner={'__typename':'PullRequest','number':config.get('destination_number',v['number']),'repository':{'nameWithOwner':config.get('thread_repo','a/b')}}
  node['issue']=owner;node['pullRequest']=owner
  body={'data':{'repository':{'pullRequest':{'headRefOid':config.get('conversation_head',config.get('head','abc'))}},'node':None if config.get('missing_destination') else node}}
 elif q.startswith('query Conversation'):
  if config.get('slow_conversation'):
   (root/'conversation-started').write_text('started');time.sleep(config['slow_conversation'])
  source='replies' if 'thread' in v else 'threads' if 'reviewThreads(first:$first' in q else 'reviews' if 'reviews(first:' in q else 'comments'
  total=config.get('conversation_count',105);offset=int(v.get('after') or 0);size=v['first'];head=config.get('conversation_head',config.get('head','abc'))
  nodes=[]
  for i in range(offset,min(total,offset+size)):
   if source=='threads':nodes.append({'id':'T_'+str(i),'path':'file-'+str(i)+'.txt','diffSide':'RIGHT','line':1,'originalLine':1,'startLine':None,'startDiffSide':None,'originalStartLine':None,'isResolved':i%2==0,'isOutdated':i%3==0,'comments':{'nodes':[{'id':'C_'+str(i),'createdAt':'2026-10-18T00:00:00Z','url':'https://github.com/a/b/pull/1#discussion_r'+str(i),'commit':{'oid':head},'originalCommit':{'oid':'b'*40}}]}})
   else:nodes.append({'id':source+'_'+str(i),'url':'https://github.com/a/b/pull/1#'+('pullrequestreview-' if source=='reviews' else 'discussion_r' if source=='replies' else 'issuecomment-')+str(i),'createdAt':'2026-10-18T00:00:00Z','author':{'login':'reader'},'body':config.get('conversation_body','hello '+str(i)),'state':'APPROVED'})
  connection={'totalCount':total,'pageInfo':{'hasNextPage':offset+size<total,'endCursor':str(offset+size) if offset+size<total else None},'nodes':nodes}
  prdata={'id':'PR_1','headRefOid':head}
  if source!='replies':prdata['reviewThreads' if source=='threads' else source]=connection
  body={'data':{'repository':{'pullRequest':prdata}}}
  if source=='replies':body['data']['node']={'id':v['thread'],'pullRequest':{'number':v['number'],'repository':{'nameWithOwner':config.get('thread_repo','a/b')}},'comments':connection}
  if config.get('deny_conversation'):body={'data':{'repository':{'pullRequest':None}},'errors':[{'type':'FORBIDDEN'}]}
 elif q.startswith('query Authored'):
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
 if endpoint.startswith('repos/a/b/actions/runs/'):
  body={'id':int(endpoint.split('/')[-1]),'repository':{'full_name':'a/b'},'run_attempt':config.get('alert_attempt',config.get('attempt',1)),'head_sha':config.get('head','abc'),'status':config.get('alert_run_status','completed'),'conclusion':'success'}
 elif endpoint=='user':body={'id':identity['databaseId'],'login':'reader'}
 elif endpoint.startswith('user/teams'):
  body=[] if config.get('no_teams') else [{'node_id':'T_1','slug':'reviewers','organization':{'login':'o'}}]
 elif endpoint.startswith('search/issues'):
  bucket='search';url=urllib.parse.urlparse(endpoint);params=urllib.parse.parse_qs(url.query);q=params['q'][0];page=int(params.get('page',['1'])[0]);size=int(params.get('per_page',['100'])[0]);ids=config.get('team_ids',[1,3]) if 'team-review-requested:' in q else config.get('direct_ids',list(range(1,config['direct_count']+1)) if 'direct_count' in config else [1,2]);
  if 'user-review-requested:' not in q and 'team-review-requested:' not in q:
   ids=config.get('query_ids',[1,2])
   if config.get('slow_query'):
    (root/'query-started').write_text(q);time.sleep(config['slow_query'])
   if config.get('query_mismatch'):(root/'mode').write_text('mismatch')
  if 'created:' in q:
   interval=q.split('created:')[1].split()[0];lo,hi=interval.split('..');lo=calendar.timegm(time.strptime(lo,'%Y-%m-%dT%H:%M:%SZ'));hi=calendar.timegm(time.strptime(hi,'%Y-%m-%dT%H:%M:%SZ'));ids=[n for n in ids if lo<=1700000000+n<=hi]
  if config.get('deny_team') and 'team-review-requested:' in q:status=403;body={'message':'Forbidden'}
  else:
   body={'total_count':len(ids),'incomplete_results':bool(config.get('incomplete')),'items':[issue(n) for n in ids[(page-1)*size:page*size]]}
  if 'user-review-requested:' not in q and 'team-review-requested:' not in q:
   if config.get('query_failure'):status=500;body={'message':'failure'}
   elif config.get('query_denied'):status=404;body={'message':'Not Found'}
   elif config.get('query_rate'):status=429;remaining=0;body={'message':'rate limit'}
   else:
    body['incomplete_results']=bool(config.get('query_partial'));body['total_count']=config.get('query_total',len(ids))
    if config.get('query_title_bytes'):
     for item in body['items']:item['title']='x'*config['query_title_bytes']
 elif '/stacks?' in endpoint:body=[]
 elif endpoint.startswith('notifications?'):
  entries=config.get('notifications',[]);params=urllib.parse.parse_qs(urllib.parse.urlparse(endpoint).query);page=int(params.get('page',['1'])[0]);size=int(params.get('per_page',['50'])[0]);body=entries[(page-1)*size:page*size]
 elif endpoint=='repos/a/b/pulls/comments/9' or any(endpoint=='repos/a/b/pulls/comments/'+str(f.get('node',{}).get('fullDatabaseId')) for f in config.get('destinations',{}).values()):
  if config.get('deny_coordinates'):status=403
  body={'node_id':config.get('destination_id','target'),'pull_request_url':'https://api.github.com/repos/a/b/pulls/1','body':config.get('coordinate_body',config.get('conversation_body','hello destination')),'path':'file-0.txt','side':'RIGHT','line':1,'position':1,'start_line':None,'start_side':None,'commit_id':config.get('head','abc'),'original_commit_id':'b'*40,'original_line':1}
  for node_id,f in config.get('destinations',{}).items():
   if endpoint=='repos/a/b/pulls/comments/'+str(f.get('node',{}).get('fullDatabaseId')):
    body.update(node_id=node_id,body=f['node']['body']);body.update(f.get('coordinates',{}))
 elif any(part in endpoint for part in ['/comments?','/reviews?','/timeline?']):
  source='timeline' if '/timeline?' in endpoint else 'reviews' if '/reviews?' in endpoint else 'review_comments' if '/pulls/' in endpoint else 'comments'
  if config.get('deny_activity')==source:status=403;body={'message':'Forbidden'}
  else:
   entries=config.get(source,[]);params=urllib.parse.parse_qs(urllib.parse.urlparse(endpoint).query);page=int(params.get('page',['1'])[0]);size=int(params.get('per_page',['50'])[0]);body=entries[(page-1)*size:page*size]
 elif '/files?' in endpoint:
  params=urllib.parse.parse_qs(urllib.parse.urlparse(endpoint).query);page=int(params.get('page',['1'])[0]);size=int(params.get('per_page',['20'])[0]);count=config.get('files_count',45)
  if config.get('slow_files'):
   (root/'files-started').write_text(str(page));time.sleep(config['slow_files'])
  if config.get('deny_files'):status=404;body={'message':'Not Found'}
  else:
   body=[{'filename':'file-'+str(i)+'.txt','status':config.get('patch_status','added'),'additions':config.get('patch_additions',1),'deletions':config.get('patch_deletions',0),'blob_url':'https://github.com/a/b/blob/'+config.get('head','abc')+'/file-'+str(i)+'.txt','patch':config.get('patch','@@ -0,0 +1 @@\n+hello \tworld')} for i in range((page-1)*size,min(count,page*size))]
   if config.get('overflow_files') and size>1:body[0]['patch']='x'*(17*1024*1024)
 elif config.get('actions_logs') and '/actions/runs/' in endpoint:
  run=int(endpoint.split('/')[-1]);body={'id':run,'run_attempt':config.get('remote_attempt',config.get('attempt',1)),'head_sha':config.get('head','abc')}
 elif config.get('actions_logs') and '/check-runs/' in endpoint:
  job=int(endpoint.split('/')[-1]);body={'node_id':config.get('log_check_node','CR_'+str(job-100)),'app':{'slug':config.get('log_provider','github-actions')},'head_sha':config.get('head','abc')}
 elif config.get('actions_logs') and '/actions/jobs/' in endpoint:
  if endpoint.endswith('/logs'):
   (root/'log-started').write_text('started')
   time.sleep(config.get('slow_log',0))
   status=config.get('log_status',200)
   text=config.get('log_text','private-log-canary <script>stay text</script>')
   if 'log_bytes' in config:text='x'*config['log_bytes']
   if config.get('change_during_log'):
    config.update(config['change_during_log']);(root/'scenario.json').write_text(json.dumps(config))
   sys.stdout.write('HTTP/2.0 '+str(status)+' Status\r\nX-RateLimit-Remaining: 4900\r\n\r\n'+text);sys.exit(0 if status<400 else 1)
  job=int(endpoint.split('/')[-1]);body={'id':job,'run_id':job-98,'run_attempt':config.get('remote_attempt',config.get('attempt',1)),'head_sha':config.get('head','abc'),'html_url':'https://github.com/a/b/actions/runs/'+str(job-98)+'/job/'+str(job),'check_run_url':'https://api.github.com/repos/a/b/check-runs/'+str(job)}
 elif endpoint.startswith('repos/a/b/pulls/'):
  n=int(endpoint.split('/')[-1]);p=pr(n);body={'node_id':p['id'],'number':n,'title':p['title'],'html_url':p['url'],'body':'<script>untrusted</script>','body_html':'&lt;script&gt;untrusted&lt;/script&gt;','created_at':'2026-10-17T00:00:00Z','changed_files':config.get('files_count',45),'base':{'sha':config.get('base_sha','b'*40),'ref':'main','repo':{'full_name':'a/b','id':1}},'head':{'sha':config.get('head','abc')},'state':p['state'].lower(),'draft':False,'user':{'login':'reader','id':42}}
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
