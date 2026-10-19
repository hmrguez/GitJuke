import {Skeleton} from '../components/Skeleton';
import {NotificationOnboarding} from '../features/notifications/NotificationOnboarding';
import {ActivityReader,ActivityDestination} from '../features/activity/Activity';
import {Preferences} from '../features/preferences/Preferences';
import {presets} from './theme';
import {friendlyError} from '../lib/ui-copy';
import React, { useCallback, useEffect, useRef, useState } from 'react';
import { GitBranch, GitPullRequest, RefreshCw, Settings2, Pause, Play, ExternalLink, ArrowLeft, Activity, Inbox, Search, FileText, FileDiff, MessageSquare, CircleCheck } from 'lucide-react';
import { command, revisions, type Overview, type Page, type Snapshot, type Status, type CheckPage, type ActivityPage, type ActivityRecord } from '../lib/ipc';
import {QueryNavigation,QueryEditor,QueryReader} from '../features/queries/SavedQueries';
import {Sidebar,useSidebarState} from '../components/Sidebar';
import {useNativeMenu} from './native-menu';
import {AddPullRequest} from '../features/pull-requests/AddPullRequest';
import {useNavigationHistory,type NavigationEntry} from './navigation-history';
import type {SavedQuery,QueryList} from '../lib/ipc';
import {Changes} from '../features/changes/Changes';
import {Conversation} from '../features/conversation/Conversation';
import {Markdown} from '../components/Markdown';
import {CheckReader} from '../features/checks/Checks';
import {PullRequestIcon,PullRequestBadge,pullRequestState,pullRequestLabel} from '../features/pull-requests/StatusIndicators';
import './app.css';
import './theme.css';
import {compactReader,readerListKey,useReaderTextSize} from './reader-accessibility';

export function App(){
  const textSize=useReaderTextSize();
  const sidebar=useSidebarState();
  const [preferencesSection,setPreferencesSection]=useState<'Appearance'|'About'>('Appearance');
  useEffect(()=>{window.addEventListener('keydown',readerListKey);return()=>window.removeEventListener('keydown',readerListKey);},[]);
  const [status,setStatus]=useState<Status|null>(null),[page,setPage]=useState<Page|null>(null);
  const [selected,setSelected]=useState<{repo:string;number:number}|null>(null);
  const [detail,setDetail]=useState<Snapshot<Overview|null>|null>(null),[error,setError]=useState('');
  const [view,setView]=useState<'mine'|'review'|'activity'|'query'>('mine'),[tab,setTab]=useState<'overview'|'changes'|'conversation'|'checks'>('overview');
  const [checks,setChecks]=useState<CheckPage|null>(null),[checkOffset,setCheckOffset]=useState(0);
  const [destination,setDestination]=useState<ActivityRecord|null>(null);
  const [notificationRecord,setNotificationRecord]=useState<ActivityRecord|null>(null);
  const [notificationUnavailable,setNotificationUnavailable]=useState(false);
  const restored=useRef(false),selectionOrigin=useRef<string|null>(null),selectionFocus=useRef<Element|null|undefined>(undefined);
  const [query,setQuery]=useState<SavedQuery|null>(null),[queryEditor,setQueryEditor]=useState(false),[editingQuery,setEditingQuery]=useState<SavedQuery|null>(null);
  const [search,setSearch]=useState('');
  const [addingPullRequest,setAddingPullRequest]=useState(false);
  const [offset,setOffset]=useState(0),[preferences,setPreferences]=useState(false);
  const readSerial=useRef(0),detailSerial=useRef(0),storeRevision=useRef(0);
  const selectedRef=useRef(selected);selectedRef.current=selected;
  const load=useCallback(async()=>{
    const request=++readSerial.current;
    try{
      const s=await command<Status>('runtime_status');
      if(request!==readSerial.current||s.revision<storeRevision.current)return;
      storeRevision.current=s.revision;setStatus(s);
      if(s.locked){setPage(null);setDetail(null);setChecks(null);setDestination(null);setNotificationRecord(null);setError('');return;}
      if(!restored.current){restored.current=true;if(s.selected){const selection={repo:s.selected[0],number:s.selected[1]};selectedRef.current=selection;setSelected(selection);}}
      if(view==='query'||view==='activity')return;
      const p=await command<Page>(view==='mine'?'authored_page':'requested_page',{offset});
      if(request!==readSerial.current||p.revision<storeRevision.current)return;
      storeRevision.current=p.revision;setPage(p);setError('');
    }catch(e){if(request===readSerial.current)setError(String(e));}
  },[offset,view]);
  const loadDetail=useCallback(async(offsetOverride?:number)=>{
    const pr=selectedRef.current;if(!pr)return;const request=++detailSerial.current;
    try{const d=await command<Snapshot<Overview|null>>('read_overview',pr);
      if(request!==detailSerial.current||selectedRef.current?.repo!==pr.repo||selectedRef.current?.number!==pr.number||d.revision<storeRevision.current)return;
      setDetail(d);
      const next=await command<CheckPage>('read_checks',{...pr,offset:offsetOverride??checkOffset});
      if(request===detailSerial.current&&selectedRef.current?.repo===pr.repo&&selectedRef.current?.number===pr.number&&next.revision>=storeRevision.current)setChecks(next);
    }catch(e){if(request===detailSerial.current)setError(String(e));}
  },[checkOffset]);
  const history=useNavigationHistory({view,query,selected,tab,destination,notificationRecord,notificationUnavailable,search,offset,checkOffset,origin:selectionOrigin.current},!!status,restoreNavigation,resolveNavigation,e=>setError(String(e)));
  async function resolveNavigation(entry:NavigationEntry){
    let next=entry;
    if(entry.view==='query'&&entry.query){
      let offset=0,found:SavedQuery|undefined;
      do{
        const list=await command<QueryList>('saved_queries',{offset});
        found=list.items.find(q=>q.id===entry.query!.id);
        if(found||list.nextOffset===null)break;
        offset=list.nextOffset;
      }while(true);
      if(!found)return null;
      next={...next,query:found};
    }
    if(entry.selected){
      const overview=await command<Snapshot<Overview|null>>('read_overview',entry.selected);
      if(!overview.data)return null;
    }
    return next;
  }
  function restoreNavigation(entry:NavigationEntry){
    ++readSerial.current;++detailSerial.current;
    setView(entry.view);setQuery(entry.query);setTab(entry.tab);setDestination(entry.destination);
    setNotificationRecord(entry.notificationRecord);setNotificationUnavailable(entry.notificationUnavailable);
    setSearch(entry.search);setOffset(entry.offset);setCheckOffset(entry.checkOffset);
    if(entry.view!==view||entry.offset!==offset)setPage(null);
    setDetail(null);setChecks(null);setError('');
    selectedRef.current=entry.selected;setSelected(entry.selected);
    selectionOrigin.current=entry.origin;selectionFocus.current=entry.selected?document.activeElement:undefined;
    if(entry.selected){
      const selection=entry.selected,request=detailSerial.current;
      void command('select_pr',selection).then(()=>{
        if(request===detailSerial.current)void loadDetail(entry.checkOffset);
      }).catch(e=>{if(request===detailSerial.current)setError(String(e));});
    }
    if(!entry.selected)requestAnimationFrame(()=>{
      const origin=entry.origin?document.getElementById(entry.origin):null;
      const target=origin??document.querySelector<HTMLElement>('.stack-navigator');
      target?.focus();
    });
  }
  useEffect(()=>{
    let disposed=false;let cleanup:(()=>void)|undefined;
    void load();void loadDetail();
    // Events are hints. Periodic authoritative rereads and focus recover lost events/reconnects.
    void revisions(()=>{void load();void loadDetail();}).then(fn=>{if(disposed)fn();else cleanup=fn;}).catch(()=>{});
    const timer=setInterval(()=>{void load();void loadDetail();},2000);
    const focus=()=>{void load();void loadDetail();};window.addEventListener('focus',focus);
    return()=>{disposed=true;cleanup?.();clearInterval(timer);window.removeEventListener('focus',focus);};
  },[load,loadDetail]);
  useEffect(()=>{
    const theme=presets[status?.settings.theme.mode??'dark'];
    for(const key of ['canvas','surface','accent','ink'] as const)document.documentElement.style.setProperty(`--theme-${key}`,theme[key]);
    document.documentElement.dataset.mode=theme.mode;
  },[status?.settings.theme]);
  async function action(name:string,args?:Record<string,unknown>){try{await command(name,args);setError('');await load();}catch(e){setError(String(e));}}
  async function select(repo:string,number:number,origin?:string){
    selectionOrigin.current=origin??(document.activeElement?.id||null);selectionFocus.current=document.activeElement;
    setDestination(null);
    ++detailSerial.current;setSelected({repo,number});selectedRef.current={repo,number};setDetail(null);setChecks(null);setCheckOffset(0);
    try{await command('select_pr',{repo,number});await loadDetail(0);return true;}catch(e){setError(String(e));return false;}
  }
  function backToList(){selectionFocus.current=undefined;const id=destination?.id;setDestination(null);setSelected(null);selectedRef.current=null;++detailSerial.current;setDetail(null);requestAnimationFrame(()=>{const origin=document.getElementById(id?`activity-${id}`:selectionOrigin.current??'');if(origin?.isConnected)origin.focus();else document.querySelector<HTMLElement>('.stack-navigator')?.focus();});}
  function browseConversation(){setDestination(null);setTab('conversation');requestAnimationFrame(()=>document.getElementById('conversation')?.focus());}
  useEffect(()=>{
    let active=true;let busy=false;
    async function route(){if(busy)return;busy=true;try{
      // null means no click; the wrapper distinguishes a summary/unavailable source.
      const result=await command<{record:ActivityRecord|null;unavailable:boolean}|null>('take_notification_route');
      if(!active||result===null)return;
      setNotificationUnavailable(result.unavailable);
      setNotificationRecord(null);
      setView('activity');setPreferences(false);setSearch('');
      if(result.record){const a=result.record;if(!await select(a.repo,a.number))return;if(!active||selectedRef.current?.repo!==a.repo||selectedRef.current?.number!==a.number)return;
        if(['comment','review','mention'].includes(a.kind)){setNotificationRecord(null);setDestination(a);setTab('conversation');}else{setDestination(null);setTab(a.kind==='check'?'checks':'conversation');setNotificationRecord(a);}
      }else{backToList();setError('');}
    }catch{/* Keep pending clicks until the original identity is validated. */}finally{busy=false;}}
    void route();const timer=setInterval(()=>void route(),1000);return()=>{active=false;clearInterval(timer);};
  },[]);
  useEffect(()=>{if(view!=='activity'||selected)setNotificationUnavailable(false);},[view,selected]);
  function chooseQuery(q:SavedQuery){if(compactReader())backToList();setDestination(null);setQuery(q);setView('query');}
  function editQuery(){setEditingQuery(query);setQueryEditor(true);}
  const pr=detail?.data;
  function navigate(item:'mine'|'review'|'activity'){
    if(compactReader())backToList();setDestination(null);setView(item);setOffset(0);setSearch('');setPage(null);
  }
  function openPreferences(section:'Appearance'|'About'='Appearance'){setPreferencesSection(section);setPreferences(true);}
  useNativeMenu({selected:!!pr&&!status?.locked,filter:view==='mine'||view==='review',collapsed:sidebar.collapsed,textSize:textSize.size,paused:status?.settings.paused??false,canMonitor:!!status&&!status.locked&&!status.storageStopped},id=>{
    const sizing=['reader-larger','reader-smaller','reader-actual'].includes(id);
    if(document.querySelector('dialog[open]')&&!sizing)return;
    switch(id){
      case 'reader-about':openPreferences('About');break;
      case 'reader-preferences':openPreferences();break;
      case 'reader-add':setAddingPullRequest(true);break;
      case 'reader-query':setEditingQuery(null);setQueryEditor(true);break;
      case 'reader-open-pr':if(pr&&selected&&!status?.locked)void action('open_pr',selected);break;
      case 'reader-filter':
        if(view!=='mine'&&view!=='review')break;
        if(compactReader())backToList();
        requestAnimationFrame(()=>document.querySelector<HTMLInputElement>('.search-field input')?.focus());break;
      case 'reader-sidebar':sidebar.setCollapsed(value=>!value);break;
      case 'reader-mine':navigate('mine');break;
      case 'reader-review':navigate('review');break;
      case 'reader-activity':navigate('activity');break;
      case 'reader-refresh':if(status&&!status.locked&&!status.storageStopped)void action('refresh');break;
      case 'reader-pause':if(status&&!status.locked&&!status.storageStopped)void action('set_pause',{paused:!status.settings.paused});break;
      case 'reader-larger':textSize.setSize(Math.min(200,textSize.size+25));break;
      case 'reader-smaller':textSize.setSize(Math.max(100,textSize.size-25));break;
      case 'reader-actual':textSize.setSize(100);break;
    }
  });
  useEffect(()=>{
    const origin=selectionFocus.current;if(origin===undefined||!pr||destination)return;
    selectionFocus.current=undefined;
    if(document.activeElement===origin||document.activeElement===document.body)document.getElementById(`${tab}-tab`)?.focus();
  },[pr,destination,tab]);
  return <div className={`app-frame ${selected?'show-detail':''}`}>
    <output className="sr-only" aria-live="polite">Text size {textSize.size}%</output>
    <Sidebar {...sidebar} history={history} onAddPullRequest={()=>setAddingPullRequest(true)}>
      <nav id="sidebar-navigation" aria-label="Primary"><p className="section-label">Workspace</p>{(['mine','review','activity'] as const).map(item=><button key={item} aria-label={item==='mine'?'My PRs':item==='review'?'Review requests':'Activity'} title={item==='mine'?'My PRs':item==='review'?'Review requests':'Activity'} className={`nav-item ${view===item?'active':''}`} aria-current={view===item?'page':undefined} onClick={()=>navigate(item)}>{item==='mine'?<GitPullRequest size={16}/>:item==='review'?<Inbox size={16}/>:<Activity size={16}/>}<span>{item==='mine'?'My PRs':item==='review'?'Review requests':'Activity'}</span></button>)}<QueryNavigation revision={status?.revision??0} current={view==='query'?query:null} onSelect={chooseQuery} onCreate={()=>{setEditingQuery(null);setQueryEditor(true);}} onEdit={editQuery}/></nav>

      <div className="sidebar-bottom">{status?.storage.warning&&<p role="status" className="state-banner">Storage is running low. Open Preferences to review it.</p>}<button className="nav-item" title="Preferences" aria-label="Preferences" onClick={()=>openPreferences()}><Settings2 size={16}/><span>Preferences</span></button>
        <div className="account" title={status?.identity?.login??'Not connected'}><span className="avatar">{status?.identity?.login.slice(0,2).toUpperCase()??'?'}</span><div>{status?.identity?.login??'Not connected'}<small>GitHub.com</small></div></div>
        <button className="monitor" data-status={!status?'unknown':status.storageStopped?'failure':status.settings.paused?'pending':'success'} title={status?.settings.paused?'Resume updates':'Pause updates'} aria-label={status?.settings.paused?'Resume updates':'Pause updates'} onClick={()=>void action('set_pause',{paused:!status?.settings.paused})} disabled={!status}>
          {status?.settings.paused?<Play size={13}/>:<Pause size={13}/>}<span>{status?.settings.paused?'Paused':status?.storageStopped?'Updates need attention':'Updates on'}<small>{status?.busy?'Checking for updates…':status?.settings.paused?'Updates paused':status?.storageStopped?'Free up space to resume':'Updates in the background'}</small></span>
        </button>
      </div>
    </Sidebar>
    <main className="stacks-workspace">
      {view==='query'&&query?<QueryReader key={`${query.id}:${query.version}`} query={query} revision={status?.revision??0} locked={status?.locked??true} health={status?.storageStopped?status.health:undefined} select={(repo,number)=>void select(repo,number)} edit={editQuery}/>:view==='activity'?<ActivityReader notificationUnavailable={notificationUnavailable} health={status?.storageStopped?status.health:undefined} locked={status?.locked??true} revision={status?.revision??0} report={setError} open={a=>{void select(a.repo,a.number,`activity-${a.id}`);setDestination(a);setTab('conversation');}}/>:<section className="stack-navigator" tabIndex={-1} aria-label="Pull requests">
        <div className="list-heading"><div><h1>{view==='mine'?'Pull requests':'Review requests'}</h1></div><button className="icon-button" aria-label="Refresh pull requests" onClick={()=>void action('refresh')}><RefreshCw size={16}/></button></div>

        {error&&<p role="alert" className="state-banner">{friendlyError(error)}</p>}
        {status?.locked&&<p className="state-banner">Check your GitHub connection in Preferences.</p>}
        {status?.storageStopped&&<p className="state-banner">Updates are paused while local storage needs attention.</p>}
        {page?.error&&<p className="state-banner">{friendlyError(page.error,'Couldn’t update pull requests. Try refreshing.')}</p>}
        <label className="search-field"><Search size={14}/><input aria-label="Filter pull requests" placeholder="Filter this page…" value={search} onChange={e=>setSearch(e.target.value)}/></label><div className="pr-list">{!page&&!status?.locked&&!error&&<Skeleton shape="pull-requests" label="Loading pull requests"/>}{page?.items.filter(p=>`${p.repo} ${p.number} ${p.title}`.toLowerCase().includes(search.toLowerCase())).map(p=><button key={p.id} id={`pr-${p.id}`} data-pr-state={pullRequestState(p)} className={`stack-node ${selected?.repo===p.repo&&selected.number===p.number?'active':''}`} onClick={()=>void select(p.repo,p.number,`pr-${p.id}`)}>
          <span className="node-meta">{p.repo} · #{p.number}</span><strong className="node-title"><PullRequestIcon pr={p} size={14}/>{p.title}</strong><span className="node-meta"><span className="pr-state-text">{pullRequestLabel(p)}</span> · {p.author??'Unavailable author'}</span>
        </button>)}</div>
        {search&&page&&page.items.length>0&&!page.items.some(p=>`${p.repo} ${p.number} ${p.title}`.toLowerCase().includes(search.toLowerCase()))&&<p className="empty">No matches on this page.</p>}
        {page&&!page.complete&&!status?.locked&&!page.error&&<Skeleton shape="pull-requests" label="Loading more pull requests" count={page.items.length?1:4}/>}
        {page?.items.length===0&&page.complete&&<p className="empty">{view==='mine'?'You have no open pull requests.':'No open review requests.'}</p>}
        <div className="pagination"><button disabled={offset===0} onClick={()=>setOffset(Math.max(0,offset-100))}>Previous</button><span>{page?.items.length?offset+1:0}–{offset+(page?.items.length??0)}</span><button disabled={page?.nextOffset==null} onClick={()=>setOffset(page?.nextOffset??0)}>Next</button></div>
      </section>}
      <article className="detail" aria-label="Pull request reader">
        {selected?<>
          <div className="detail-toolbar"><button className="back-to-list" onClick={backToList}><ArrowLeft size={14}/>Back</button><span>{selected.repo} / #{selected.number}</span><button disabled={!pr} onClick={()=>void action('open_pr',selected)}><ExternalLink size={13}/>Open in GitHub</button></div>
          {detail?.error&&<p className="state-banner">{friendlyError(detail.error,'Couldn’t update this pull request. Try refreshing.')}</p>}
          {pr?<><header className="pr-heading"><PullRequestBadge pr={checks?.pr??pr}/><h1>{checks?.pr?.title??pr.title}</h1><p className="pr-byline">{pr.author??'Unknown author'}</p></header>
            {pr.stack.state==='published'&&<section className="stack-section" aria-label="Pull request stack"><GitBranch size={16}/><div><span>Stack #{pr.stack.number}</span><ol reversed>{[...pr.stack.members].reverse().map(member=><li key={member.number}><button aria-current={member.number===pr.number?'page':undefined} onClick={()=>void select(pr.repo,member.number)}>#{member.number} {member.title}</button></li>)}</ol></div></section>}
            {checks?.head&&checks.head!==pr.head&&<p className="state-banner">This pull request has new changes. <button onClick={()=>void select(pr.repo,pr.number)}>Update overview</button></p>}
            <div className="tabs" role="tablist" aria-label="Reader">{(['overview','changes','conversation','checks'] as const).map(item=><button key={item} role="tab" aria-selected={tab===item} id={`${item}-tab`} aria-controls={item} onClick={()=>{if(item!=='changes'&&item!=='conversation')setDestination(null);setTab(item);}} onKeyDown={e=>{if(['ArrowLeft','ArrowRight','Home','End'].includes(e.key)){e.preventDefault();const tabs=['overview','changes','conversation','checks'] as const;const next=e.key==='Home'?tabs[0]:e.key==='End'?tabs[3]:tabs[(tabs.indexOf(tab)+(e.key==='ArrowRight'?1:3))%4];if(next!=='changes'&&next!=='conversation')setDestination(null);setTab(next);document.getElementById(`${next}-tab`)?.focus();}}} tabIndex={tab===item?0:-1}>{item==='overview'?<FileText size={14}/>:item==='changes'?<FileDiff size={14}/>:item==='conversation'?<MessageSquare size={14}/>:<CircleCheck size={14}/>} {item==='overview'?'Overview':item==='changes'?'Changes':item==='conversation'?'Conversation':'Checks'}</button>)}</div>
            {tab==='overview'?<section className="overview" id="overview" tabIndex={0} role="tabpanel" aria-labelledby="overview-tab">{pr.bodyLimited?<p>This description is too long to preview. View it on GitHub.</p>:<Markdown baseUrl={pr.url}>{pr.body||'No description.'}</Markdown>}</section>:tab==='changes'?<Changes key={`${pr.repo}:${pr.number}`} pr={pr} currentHead={checks?.head??pr.head} destination={destination?.id.slice(`github.com:${pr.repo}:${pr.number}:`.length)} onBack={backToList} onBrowse={browseConversation} paged={status?.settings.pagedChanges??false} revision={status?.revision??0} onMode={paged=>void action('set_changes_mode',{paged})}/>:tab==='conversation'?<Conversation key={`${pr.repo}:${pr.number}:${pr.head}:${destination?.id??'ordinary'}`} destination={destination?.id.startsWith(`github.com:${pr.repo}:${pr.number}:`)?destination.id.slice(`github.com:${pr.repo}:${pr.number}:`.length):undefined} onBack={backToList} onBrowse={browseConversation} pr={pr} currentHead={checks?.head??pr.head} revision={status?.revision??0}/>:<CheckReader revision={status?.revision??0} repo={pr.repo} number={pr.number} checks={checks} offset={checkOffset} setOffset={setCheckOffset}/>}

          </>:detail?.error?<div className="empty">This PR is unavailable.</div>:<Skeleton shape="detail" label="Loading pull request"/>}
        </>:<div className="empty reader-empty"><GitPullRequest size={36}/><h1>Select a pull request</h1></div>}
      </article>
    </main>
    {status&&!status.settings.alerts?.onboarded&&<NotificationOnboarding settings={status.settings.alerts} action={action}/>}
    {notificationRecord&&!status?.locked&&<ActivityDestination key={notificationRecord.id} record={notificationRecord} close={()=>setNotificationRecord(null)} report={setError}/>}
    {queryEditor&&<QueryEditor query={editingQuery} close={()=>setQueryEditor(false)} onSaved={q=>{setQueryEditor(false);chooseQuery(q);requestAnimationFrame(()=>document.querySelector<HTMLElement>('[aria-label="Saved query results"]')?.focus());void load();}} onDeleted={()=>{setQueryEditor(false);setQuery(null);setView('mine');void load();}}/>}
    {addingPullRequest&&<AddPullRequest open={async(repo,number)=>{const opened=await select(repo,number);if(opened)setTab('overview');return opened;}} close={()=>{setAddingPullRequest(false);requestAnimationFrame(()=>document.querySelector<HTMLButtonElement>('button[aria-label="Add pull request"]')?.focus());}}/>}
    {preferences&&<Preferences initialSection={preferencesSection} textSize={textSize} status={status} error={error} close={()=>{setPreferences(false);requestAnimationFrame(()=>document.querySelector<HTMLButtonElement>('button[title="Preferences"]')?.focus());}} action={action}/>}
  </div>;
}
