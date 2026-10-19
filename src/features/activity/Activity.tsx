import {Skeleton} from '../../components/Skeleton';
import React,{useEffect,useRef,useState} from 'react';
import {Activity,X} from 'lucide-react';
import {command,type ActivityPage,type ActivityRecord} from '../../lib/ipc';
import {friendlyError,activityLabel} from '../../lib/ui-copy';
import {Markdown} from '../../components/Markdown';

export function ActivityReader({locked,revision,report,health,open,notificationUnavailable}:{notificationUnavailable?:boolean;locked:boolean;revision:number;report:(s:string)=>void;health?:string;open:(a:ActivityRecord)=>void}){
  const region=useRef<HTMLElement>(null);
  const [offset,setOffset]=useState(0),[page,setPage]=useState<ActivityPage|null>(null),[opened,setOpened]=useState<ActivityRecord|null>(null);
  useEffect(()=>{let active=true;if(locked){setPage(null);setOpened(null);return;}
    void command<ActivityPage>('activity_page',{offset}).then(p=>{if(active&&p.revision>=revision){setPage(p);setOpened(current=>current?(p.items.find(a=>a.id===current.id)??null):null);}}).catch(e=>{if(active)report(String(e));});return()=>{active=false;};
  },[offset,revision,locked,report]);
  return <section ref={region} className="stack-navigator" tabIndex={-1} aria-label="Personal activity"><div className="list-heading"><div><h1>Activity</h1></div><Activity size={17}/></div>
    {notificationUnavailable&&!locked&&<p className="state-banner" role="status">This notification is no longer available. You can still browse Activity.</p>}

    {health&&<p className="state-banner" role="status">{friendlyError(health)}</p>}
    {page?.error&&<p className="state-banner" role="status">{friendlyError(page.error,'Couldn’t update Activity. Try again shortly.')}</p>}
    <button disabled={!page?.items.length} onClick={()=>{const ids=page?.items.map(a=>a.id)??[];void command('activity_seen',{ids}).catch(e=>report(String(e)));}}>Mark page as read</button>

    <div className="pr-list">{page?.items.map(a=><button key={a.id} id={`activity-${a.id}`} className="stack-node" onClick={()=>{if(['comment','review','mention'].includes(a.kind))open(a);else setOpened(a);}}><span className="node-meta">{a.repo} · #{a.number} · {a.seen?'Read':'Unread'}</span><strong>{activityLabel(a.kind)} · {a.actor??'Unknown author'}</strong><span className="node-meta">{new Date(a.occurred*1000).toLocaleString()}</span></button>)}</div>
    {!locked&&!page?.error&&!health&&(!page?.baseline||page.recovery)&&<Skeleton shape="activity" label="Loading activity" count={page?.items.length?1:4}/>}
    {!page?.items.length&&(locked||page?.baseline&&!page.recovery)&&<p className="empty">{locked?'Check your GitHub connection in Preferences.':'You’re all caught up.'}</p>}
    <div className="pagination"><button disabled={offset===0} onClick={()=>{setOffset(Math.max(0,offset-100));region.current?.focus();}}>Previous</button><button disabled={page?.nextOffset==null} onClick={()=>{setOffset(page?.nextOffset??0);region.current?.focus();}}>Next</button></div>
    {opened&&!locked&&<ActivityDestination key={opened.id} record={opened} close={()=>setOpened(null)} report={report}/>}
  </section>;
}
export function ActivityDestination({record,close,report}:{record:ActivityRecord;close:()=>void;report:(s:string)=>void}){
  const dialog=useRef<HTMLDialogElement>(null);
  useEffect(()=>{dialog.current?.showModal();
    // This effect runs after this explicitly opened source destination has committed to the DOM.
    // An evicted/unavailable payload is not a successful source render.
    if(record.text!==null)void command('activity_rendered',{id:record.id,text:record.text}).catch(e=>report(String(e)));
  },[record.id,record.text,report]);
  return <dialog ref={dialog} onClose={close} aria-labelledby="activity-title" className="dialog"><div className="dialog-header"><h2 id="activity-title">{activityLabel(record.kind)}</h2><button onClick={()=>dialog.current?.close()} aria-label="Close activity"><X size={17}/></button></div><p>{record.repo} · #{record.number} · {record.actor??'Unknown author'}</p>{record.text!==null?<div tabIndex={0} role="region" aria-label="Activity source body"><Markdown baseUrl={`https://github.com/${record.repo}/pull/${record.number}`}>{record.text||'No additional details.'}</Markdown></div>:<p className="state-banner">This update can’t be previewed. Open the pull request on GitHub to see it.</p>}<p className="reader-note">{new Date(record.occurred*1000).toLocaleString()}</p></dialog>;
}
