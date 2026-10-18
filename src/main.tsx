import React, { useCallback, useEffect, useRef, useState } from 'react';
import { createRoot } from 'react-dom/client';
import { GitBranch, GitPullRequest, Layers3, RefreshCw, Settings2, Pause, Play, ExternalLink, ArrowLeft } from 'lucide-react';
import { command, presets, revisions, type Overview, type Page, type Snapshot, type Status, type Theme, type CheckPage, type WatchPage, type ActivityPage, type ActivityRecord } from './api';
import '@fontsource/jetbrains-mono/latin-400.css';
import '@fontsource/jetbrains-mono/latin-500.css';
import '@fontsource/jetbrains-mono/latin-600.css';
import './app.css';
import './theme.css';

function App(){
  const [status,setStatus]=useState<Status|null>(null),[page,setPage]=useState<Page|null>(null);
  const [selected,setSelected]=useState<{repo:string;number:number}|null>(null);
  const [detail,setDetail]=useState<Snapshot<Overview|null>|null>(null),[error,setError]=useState('');
  const [view,setView]=useState<'mine'|'review'|'activity'>('mine'),[tab,setTab]=useState<'overview'|'checks'>('overview');
  const [checks,setChecks]=useState<CheckPage|null>(null),[checkOffset,setCheckOffset]=useState(0);
  const restored=useRef(false);
  const [offset,setOffset]=useState(0),[preferences,setPreferences]=useState(false);
  const readSerial=useRef(0),detailSerial=useRef(0),storeRevision=useRef(0);
  const selectedRef=useRef(selected);selectedRef.current=selected;
  const load=useCallback(async()=>{
    const request=++readSerial.current;
    try{
      const s=await command<Status>('runtime_status');
      if(request!==readSerial.current||s.revision<storeRevision.current)return;
      storeRevision.current=s.revision;setStatus(s);
      if(s.locked){setPage(null);setDetail(null);setChecks(null);setError('');return;}
      if(!restored.current&&s.selected){restored.current=true;const selection={repo:s.selected[0],number:s.selected[1]};selectedRef.current=selection;setSelected(selection);}
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
    const theme=status?.settings.theme??presets.dark;
    for(const key of ['canvas','surface','accent','ink'] as const)document.documentElement.style.setProperty(`--theme-${key}`,theme[key]);
    document.documentElement.dataset.mode=theme.mode;
  },[status?.settings.theme]);
  async function action(name:string,args?:Record<string,unknown>){try{await command(name,args);setError('');await load();}catch(e){setError(String(e));}}
  async function select(repo:string,number:number){
    ++detailSerial.current;setSelected({repo,number});selectedRef.current={repo,number};setDetail(null);setChecks(null);setCheckOffset(0);
    try{await command('select_pr',{repo,number});await loadDetail(0);}catch(e){setError(String(e));}
  }
  const pr=detail?.data;
  return <div className={`app-frame ${selected?'show-detail':''}`}>
    <aside className="sidebar">
      <div className="brand"><span className="brand-icon"><Layers3 size={19}/></span><span>gitjuke<span className="brand-dot">.</span></span></div>
      <nav aria-label="Primary">{(['mine','review','activity'] as const).map(item=><button key={item} className={`nav-item ${view===item?'active':''}`} aria-current={view===item?'page':undefined} onClick={()=>{setView(item);setOffset(0);setPage(null);}}><GitPullRequest size={16}/><span>{item==='mine'?'My PRs':item==='review'?'Review requests':'Activity'}</span></button>)}</nav>
      <p className="implementation-note">Watches + head/check snapshots<br/>Personal activity · alerts unavailable</p>
      <div className="sidebar-bottom">{status?.storage.warning&&<p role="status" className="state-banner">Local storage ≥1 GiB. Review Preferences.</p>}<button className="nav-item" onClick={()=>setPreferences(true)}><Settings2 size={16}/><span>Preferences</span></button>
        <div className="account"><span className="avatar">{status?.identity?.login.slice(0,2).toUpperCase()??'?'}</span><div>{status?.identity?.login??'Stored gh login'}<small>GitHub.com</small></div></div>
        <button className="monitor" onClick={()=>void action('set_pause',{paused:!status?.settings.paused})} disabled={!status}>
          {status?.settings.paused?<Play size={13}/>:<Pause size={13}/>}<span>{status?.settings.paused?'Paused':status?.storageStopped?'Storage protection':'Monitoring'}<small>{status?.busy?'Reading…':`${status?.watchCount??0} watches · ${status?.delayedWatches??0} delayed`}</small></span>
        </button>
      </div>
    </aside>
    <main className="stacks-workspace">
      {view==='activity'?<ActivityReader health={status?.storageStopped?status.health:undefined} locked={status?.locked??true} revision={status?.revision??0} report={setError}/>:<section className="stack-navigator" aria-label="Pull requests">
        <div className="list-heading"><div><span className="eyebrow">YOUR WORK</span><h1>{view==='mine'?'Pull requests':'Review requests'}</h1></div><button className="icon-button" aria-label="Refresh pull requests" onClick={()=>void action('refresh')}><RefreshCw size={16}/></button></div>
        <p className="list-subhead">{page?.fetchedAt?`Cached ${new Date(page.fetchedAt*1000).toLocaleTimeString()}`:'No successful sync yet'}{page&&!page.complete?' · incomplete':''}</p>
        {error&&<p role="alert" className="state-banner">{error}</p>}
        <p className="runtime-health" role="status">{status?.health??'Connecting to native runtime…'}</p>
        {status?.nextRemoteAt&&status.nextRemoteAt>Date.now()/1000?<p className="state-banner">GitHub deadline: {new Date(status.nextRemoteAt*1000).toLocaleTimeString()}</p>:null}
        {page?.error&&<p className="state-banner">{page.error}</p>}
        <div className="pr-list">{page?.items.map(p=><button key={p.id} className={`stack-node ${selected?.repo===p.repo&&selected.number===p.number?'active':''}`} onClick={()=>void select(p.repo,p.number)}>
          <span className="node-meta">{p.repo} · #{p.number}</span><strong>{p.title}</strong><span className="node-meta">{p.draft?'Draft':p.state} · {p.author??'Unavailable author'}</span>
        </button>)}</div>
        {page?.items.length===0&&<p className="empty">{page.complete?(view==='mine'?'No open authored PRs.':'No open review requests.'):'No cached results; successful discovery is pending.'}</p>}
        <div className="pagination"><button disabled={offset===0} onClick={()=>setOffset(Math.max(0,offset-100))}>Previous</button><span>{page?.items.length?offset+1:0}–{offset+(page?.items.length??0)}</span><button disabled={page?.nextOffset==null} onClick={()=>setOffset(page?.nextOffset??0)}>Next</button></div>
        <form className="open-pr" onSubmit={e=>{e.preventDefault();const form=new FormData(e.currentTarget);void select(String(form.get('repo')),Number(form.get('number')));}}><label>Open a known PR<input name="repo" placeholder="owner/repository" required aria-label="Repository owner and name"/></label><input name="number" type="number" min="1" required placeholder="PR number" aria-label="Pull request number"/><button type="submit">Read PR</button></form>
      </section>}
      <article className="detail" aria-label="Pull request reader">
        {selected?<>
          <div className="detail-toolbar"><button className="back-to-list" onClick={()=>{setSelected(null);selectedRef.current=null;setDetail(null);}}><ArrowLeft size={14}/>Back</button><span>{selected.repo} / #{selected.number}</span><button disabled={!pr} onClick={()=>void action('open_pr',selected)}><ExternalLink size={13}/>Open in GitHub</button></div>
          {detail?.error&&<p className="state-banner">{detail.error}</p>}
          {pr?<><header className="pr-heading"><span className="open-badge">{(checks?.pr??pr).draft?'Draft':(checks?.pr??pr).state}</span><h1>{checks?.pr?.title??pr.title}</h1><p className="pr-byline">{pr.author??'Unavailable author'} · <code>{pr.base}</code> · head <code>{pr.head.slice(0,10)}</code></p></header>
            <section className="stack-section" aria-label="Published native stack"><GitBranch size={16}/>{pr.stack.state==='published'?<div><span>Stack #{pr.stack.number} · trunk {pr.stack.trunk}</span><ol>{pr.stack.members.map(member=><li key={member.number}><button aria-current={member.number===pr.number?'page':undefined} onClick={()=>void select(pr.repo,member.number)}>#{member.number} {member.title}</button></li>)}</ol></div>:<p>{pr.stack.state==='none'?'Confirmed: this PR has no published native stack.':`Native stack unknown: ${pr.stack.reason}`}</p>}</section>
            {checks?.head&&checks.head!==pr.head&&<p className="state-banner">Overview is from old head {pr.head.slice(0,10)}. Current head: {checks.head.slice(0,10)}. <button onClick={()=>void select(pr.repo,pr.number)}>Update overview</button></p>}
            <div className="tabs" role="tablist" aria-label="Reader">{(['overview','checks'] as const).map(item=><button key={item} role="tab" aria-selected={tab===item} id={`${item}-tab`} aria-controls={item} onClick={()=>setTab(item)} onKeyDown={e=>{if(e.key==='ArrowLeft'||e.key==='ArrowRight'){e.preventDefault();const next=tab==='overview'?'checks':'overview';setTab(next);document.getElementById(`${next}-tab`)?.focus();}}} tabIndex={tab===item?0:-1}>{item==='overview'?'Overview':'Checks'}</button>)}</div>
            {tab==='overview'?<section className="overview" id="overview" role="tabpanel" aria-labelledby="overview-tab">{pr.bodyLimited?<p>Description exceeds the 128 KiB local body limit. Open the PR in GitHub.</p>:<pre className="description">{pr.body??'No description.'}</pre>}<p className="reader-note">Description is displayed as safe plain text. Changes, discussions, logs are still pending implementation.</p></section>:<CheckReader checks={checks} offset={checkOffset} setOffset={setCheckOffset}/>}
            <footer className="detail-footer">{detail.fetchedAt?`Cached ${new Date(detail.fetchedAt*1000).toLocaleString()}`:'Not fetched'} · revision {detail.revision}</footer>
          </>:<div className="empty">{detail?.error?'This PR is unavailable.':'Reading the authorized PR snapshot…'}</div>}
        </>:<div className="empty reader-empty"><Layers3 size={30}/><h1>Your work, in context.</h1><p>Select a pull request to read its overview and published stack.</p></div>}
      </article>
    </main>
    {preferences&&<Preferences status={status} close={()=>setPreferences(false)} action={action}/>}
  </div>;
}
function ActivityReader({locked,revision,report,health}:{locked:boolean;revision:number;report:(s:string)=>void;health?:string}){
  const [offset,setOffset]=useState(0),[page,setPage]=useState<ActivityPage|null>(null),[opened,setOpened]=useState<ActivityRecord|null>(null);
  useEffect(()=>{let active=true;if(locked){setPage(null);setOpened(null);return;}
    void command<ActivityPage>('activity_page',{offset}).then(p=>{if(active&&p.revision>=revision){setPage(p);setOpened(current=>current?(p.items.find(a=>a.id===current.id)??null):null);}}).catch(e=>{if(active)report(String(e));});return()=>{active=false;};
  },[offset,revision,locked,report]);
  return <section className="stack-navigator" aria-label="Personal activity"><div className="list-heading"><h1>Activity</h1></div>
    {health&&<p className="state-banner" role="status">{health}</p>}
    <p className="list-subhead">{page?.baseline?'Historical baseline established':'Historical baseline pending'}{page?.recovery?` · Recovery incomplete · ${page.pending} PRs pending`:''}</p>
    <p className="reader-note">Records expire after 90 days, including unseen items. Native alerts and external-suite retry activity are unavailable in this build.</p>
    {page?.error&&<p className="state-banner" role="status">{page.error}</p>}
    <button disabled={!page?.items.length} onClick={()=>{const ids=page?.items.map(a=>a.id)??[];void command('activity_seen',{ids}).catch(e=>report(String(e)));}}>Mark loaded page seen</button>
    <p className="reader-note">Applies to these {page?.items.length??0} loaded records only.</p>
    <div className="pr-list">{page?.items.map(a=><button key={a.id} className="stack-node" onClick={()=>setOpened(a)}><span className="node-meta">{a.repo} · #{a.number} · {a.seen?'Seen':'Unseen'}</span><strong>{a.kind} · {a.actor??'Actor unavailable'}</strong><span className="node-meta">{new Date(a.occurred*1000).toLocaleString()} · {a.eligible?'Alert eligible':a.reason}</span></button>)}</div>
    {!page?.items.length&&<p className="empty">{locked?'Original identity validation required.':page?.baseline?'No retained personal activity.':'Waiting for complete source hydration.'}</p>}
    <div className="pagination"><button disabled={offset===0} onClick={()=>setOffset(Math.max(0,offset-100))}>Previous</button><button disabled={page?.nextOffset==null} onClick={()=>setOffset(page?.nextOffset??0)}>Next</button></div>
    {opened&&!locked&&<ActivityDestination key={opened.id} record={opened} close={()=>setOpened(null)} report={report}/>}
  </section>;
}
function ActivityDestination({record,close,report}:{record:ActivityRecord;close:()=>void;report:(s:string)=>void}){
  const dialog=useRef<HTMLDialogElement>(null);
  useEffect(()=>{dialog.current?.showModal();
    // This effect runs after this explicitly opened source destination has committed to the DOM.
    // An evicted/unavailable payload is not a successful source render.
    if(record.text!==null)void command('activity_seen',{ids:[record.id]}).catch(e=>report(String(e)));
  },[record.id,record.text,report]);
  return <dialog ref={dialog} onClose={close} className="dialog"><div className="dialog-header"><h2>{record.kind}</h2><button onClick={()=>dialog.current?.close()} aria-label="Close activity">×</button></div><p>{record.repo} · #{record.number} · {record.actor??'Actor unavailable'}</p>{record.text!==null?<pre className="description">{record.text||'No text supplied by this source record.'}</pre>:<p className="state-banner">Source text is unavailable or exceeds the local preview limit. This item remains unseen.</p>}{record.head&&<p className="reader-note">Source head: <code>{record.head}</code></p>}<p className="reader-note">{new Date(record.occurred*1000).toLocaleString()}</p></dialog>;
}
function CheckReader({checks,offset,setOffset}:{checks:CheckPage|null;offset:number;setOffset:(n:number)=>void}){
  return <section id="checks" className="overview" role="tabpanel" aria-labelledby="checks-tab"><h2>Head checks</h2>{checks?.error&&<p className="state-banner">{checks.error} · cached results remain labeled by revision.</p>}<p className="dialog-note">{checks?.head??'Head snapshot pending'} · review {checks?.review??'unknown'}</p>
    {checks?.commits.map(c=><p className="dialog-note" key={c.resource}>{c.resource.slice(0,10)} · {c.complete?'Complete cached contexts':'Incomplete contexts'} · {c.fetchedAt?new Date(c.fetchedAt*1000).toLocaleTimeString():'Not synced'} {c.error??''}</p>)}
    {!checks?.items.length&&<p className="dialog-note">{checks?.commits.length&&checks.commits.every(c=>c.complete)?'No supplied check contexts. This does not infer a workflow completion.':'Check reconciliation pending.'}</p>}
    {checks?.items.map(c=><div className="check-row" key={`${c.commit}:${c.id}`}><strong>{c.name}</strong><span>{c.conclusion??c.status}</span><small>{c.kind==='CheckRun'?'Check run':'Legacy status'} · {c.commit.slice(0,10)}{c.runId?` · run ${c.runId} · attempt ${c.attempt??'unknown'}`:''}</small></div>)}
    <div className="pagination"><button disabled={offset===0} onClick={()=>setOffset(Math.max(0,offset-100))}>Previous checks</button><span>{checks?.items.length?offset+1:0}–{offset+(checks?.items.length??0)} loaded</span><button disabled={checks?.nextOffset==null} onClick={()=>setOffset(checks?.nextOffset??0)}>Next checks</button></div>
  </section>;
}
function WatchInspector({revision}:{revision:number}){
  const [offset,setOffset]=useState(0),[page,setPage]=useState<WatchPage|null>(null);
  useEffect(()=>{let active=true;void command<WatchPage>('watch_page',{offset}).then(p=>{if(active)setPage(p);}).catch(()=>{if(active)setPage(null);});return()=>{active=false;};},[offset,revision]);
  return <details><summary>Active watch reasons and freshness</summary>{page?.items.map(w=><p className="dialog-note" key={w.id}>{w.repo} #{w.number} · {w.reasons.map(s=>s.startsWith('team:')?'team request':s).join(', ')} · {w.refreshedAt?new Date(w.refreshedAt*1000).toLocaleTimeString():'Not refreshed'} {w.error??''}</p>)}<div className="pagination"><button disabled={offset===0} onClick={()=>setOffset(Math.max(0,offset-100))}>Previous watches</button><button disabled={page?.nextOffset==null} onClick={()=>setOffset(page?.nextOffset??0)}>Next watches</button></div></details>;
}
function Preferences({status,close,action}:{status:Status|null;close:()=>void;action:(name:string,args?:Record<string,unknown>)=>Promise<void>}){
  const dialog=useRef<HTMLDialogElement>(null);
  useEffect(()=>{dialog.current?.showModal();},[]);
  const theme=status?.settings.theme??presets.dark;
  return <dialog ref={dialog} onClose={close} className="dialog"><div className="dialog-header"><h2>Preferences</h2><button onClick={()=>dialog.current?.close()} aria-label="Close preferences">×</button></div>
    <h3>Appearance</h3><div className="theme-presets">{(['dark','light'] as const).map(mode=><button key={mode} aria-pressed={theme.mode===mode} onClick={()=>void action('set_theme',{theme:presets[mode]})}>{mode==='dark'?'Carbon · dark':'Parchment · light'}</button>)}</div>
    {(['canvas','surface','accent','ink'] as const).map(key=><label className="color-field" key={key}>{({canvas:'Background',surface:'Surface',accent:'Accent',ink:'Text'})[key]}<input type="color" value={theme[key]} onChange={e=>void action('set_theme',{theme:{...theme,[key]:e.target.value} satisfies Theme})}/><code>{theme[key]}</code></label>)}
    <h3>Monitoring</h3><p className="dialog-note">{status?.watchCount??0} active watches · {status?.delayedWatches??0} delayed. Discovery targets 60 seconds; head/check eligibility targets 10 seconds. Activity hydrates individual comments, reviews, requests and mentions; native alerts are unavailable.</p>
    {status?.discovery.map(h=><p key={h.resource} className="dialog-note">{h.resource==='authored'?'Authored':h.resource==='direct'?'Direct requests':h.resource==='teams'?'Team membership':'Team requests'}: {h.complete?'Complete cached set':'Reconciliation pending'} · {h.fetchedAt?new Date(h.fetchedAt*1000).toLocaleTimeString():'Not synced'} {h.error??''}</p>)}
    {status?.discoveryMore&&<p className="dialog-note">Showing the first 100 source statuses.</p>}
    {status?.budgets.map(b=><p key={b.resource} className="dialog-note">{b.resource}: {b.requests} requests · {b.payloadBytes} bytes · {b.cost} points · {b.remaining} remaining {b.blockedUntil>Date.now()/1000?`· delayed until ${new Date(b.blockedUntil*1000).toLocaleTimeString()}`:''}</p>)}
    <WatchInspector revision={status?.revision??0}/>
    <h3>Stored gh context</h3><p className="dialog-note">{status?.ghVersion??'CLI not validated'} · {status?.identity?`github.com / ${status.identity.login} / ${status.identity.id}`:'Identity not validated'}</p>
    <form key={`${status?.ghPath}:${status?.configDir}`} onSubmit={e=>{e.preventDefault();const f=new FormData(e.currentTarget);void action('configure_gh',{path:String(f.get('path')),config:String(f.get('config'))});}}>
      <label>Executable<input name="path" defaultValue={status?.ghPath??'/opt/homebrew/bin/gh'} required/></label><label>Configuration directory<input name="config" defaultValue={status?.configDir??''} required/></label><button type="submit">Validate original identity</button>
    </form>
    <p className="dialog-note">GitJuke uses stored authentication. It never logs in, switches accounts or changes scopes. {status?.ignoredOverrides.length?`Ignored environment overrides: ${status.ignoredOverrides.join(', ')}.`:''}</p>
    <h3>Local storage</h3>
    {status&&<><p className="dialog-note">{(status.storage.managedBytes/1048576).toFixed(1)} MiB database and sidecars · {(status.storage.disposableBytes/1048576).toFixed(1)} / {status.storage.contentLimitBytes/1048576} MiB disposable content</p>
      <p className="dialog-note">Database {(status.storage.databaseBytes/1048576).toFixed(1)} MiB · WAL {(status.storage.walBytes/1048576).toFixed(1)} MiB · shared memory and journal {((status.storage.sharedMemoryBytes+status.storage.journalBytes)/1048576).toFixed(1)} MiB</p>
      {status.storage.warning&&<p role="status" className="state-banner">Local storage has reached {status.storage.warningAtBytes/1073741824} GiB. Durable history grows over time; clearing downloads preserves its replay prevention records.</p>}
      {status.storageStopped&&<p role="status" className="state-banner">Storage protection stops remote reads and alerts. Cached reading remains available after identity validation. Local changes require a successful write; repair is checked each minute.</p>}</>}
    <p className="dialog-note">File sizes cover the database, staged data and named SQLite sidecars. Transient SQLite sort files are not yet measured. Clearing downloads preserves identity, settings, Pause and durable history. It cannot erase growing identity, seen or delivery ledgers.</p><button onClick={()=>void action('clear_downloads')}>Clear downloaded content</button>
    <p className="dialog-note">GitJuke 0.1.0 development · macOS 15+ / Apple Silicon target. Native alerts and launch at login are not available in this slice; no login registration or permission prompt occurs.</p>
  </dialog>;
}
createRoot(document.getElementById('root')!).render(<App/>);
