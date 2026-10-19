import {Skeleton} from '../../components/Skeleton';
import React,{useEffect,useRef,useState} from 'react';
import {listen} from '@tauri-apps/api/event';
import {command,type CheckContext} from '../../lib/ipc';
import './logs.css';
export type LogJob={repo:string;number:number;head:string;check:string;offset:number};
type LogRead={state:string;text:string|null};
const labels:Record<string,string>={denied:'You don’t have access to this log.',expired:'This log is no longer available.',missing:'This log isn’t available.',empty:'This log is empty.',oversized:'This log is too long to preview. View details to read it.',failed:'Couldn’t load this log. Close and reopen it to try again.',released:'Log closed. Reopen it to continue.'};
export function CheckLog({job,check,revision}:{job:LogJob;check:CheckContext;revision:number}){
  const [open,setOpen]=useState(false),[error,setError]=useState('');
  useEffect(()=>{const change=()=>setOpen(false);window.addEventListener('gitjuke-log-selection',change);return()=>window.removeEventListener('gitjuke-log-selection',change);},[]);
  useEffect(()=>{if(!open)return;let disposed=false,stop:(()=>void)|undefined;void listen('gitjuke-reader-closed',()=>setOpen(false)).then(fn=>{if(disposed)fn();else stop=fn;}).catch(()=>{});return()=>{disposed=true;stop?.();};},[open]);
  const actions=check.kind==='CheckRun'&&!!check.runId&&!!check.attempt&&new RegExp(`^https://github\\.com/${job.repo.replace(/[.*+?^${}()|[\]\\]/g,'\\$&')}/actions/runs/${check.runId}/job/[0-9]+$`).test(check.url??'');
  return <div><div className="pagination">{actions&&<button onClick={()=>{const next=!open;window.dispatchEvent(new window.Event('gitjuke-log-selection'));setError('');setOpen(next);}}>{open?'Hide log':'Show log'}</button>}{check.url&&<button onClick={()=>void command('open_check_details',{job}).catch(e=>setError(String(e)))}>View details ↗</button>}</div>{error&&<p role="alert">Couldn’t open check details. Try again.</p>}{open&&<LogBody job={job} revision={revision}/>}</div>;
}
export function LogBody({job,revision}:{job:LogJob;revision:number}){
  const [value,setValue]=useState<LogRead>({state:'queued',text:null}),[page,setPage]=useState(0);
  const textRegion=useRef<HTMLPreElement>(null);
  const turn=(page:number)=>{setPage(page);if(textRegion.current){textRegion.current.scrollTop=0;textRegion.current.focus();}};
  const poll=useRef<(()=>void)|undefined>(undefined),version=useRef(revision),priorVersion=useRef(revision);version.current=revision;
  useEffect(()=>{
    let disposed=false,seq:number|undefined,busy=false;
    const read=async()=>{if(disposed||seq===undefined||busy)return;busy=true;const started=version.current;try{const v=await command<LogRead>('read_logs',{seq});if(!disposed&&started===version.current)setValue(v);}catch{if(!disposed)setValue({state:'failed',text:null});}finally{busy=false;if(!disposed&&started!==version.current)void read();}};
    poll.current=()=>void read();
    void command<number>('request_logs',{job}).then(n=>{seq=n;if(disposed)void command('cancel_logs',{seq:n}).catch(()=>{});else void read();}).catch(()=>{if(!disposed)setValue({state:'failed',text:null});});
    const timer=setInterval(()=>void read(),500);
    const hide=()=>{if(document.visibilityState==='hidden'){setValue({state:'released',text:null});if(seq!==undefined)void command('cancel_logs',{seq}).catch(()=>{});}};
    document.addEventListener('visibilitychange',hide);
    return()=>{disposed=true;poll.current=undefined;clearInterval(timer);document.removeEventListener('visibilitychange',hide);if(seq!==undefined)void command('cancel_logs',{seq}).catch(()=>{});};
  },[job.repo,job.number,job.head,job.check,job.offset]);
  useEffect(()=>{if(priorVersion.current!==revision){priorVersion.current=revision;setValue({state:'reading',text:null});}poll.current?.();},[revision]);
  // Keep only one bounded source and render at most 200 logical lines, as plain text.
  const lines=value.text?.split('\n')??[],pages=Math.max(1,Math.ceil(lines.length/200)),active=Math.min(page,pages-1);
  return <section aria-label="Actions job log">{value.state==='queued'||value.state==='reading'?<Skeleton shape="log" label="Loading log"/>:value.state!=='complete'&&<p role="status">{labels[value.state]??'This log isn’t available.'}</p>}{value.state==='complete'&&<><pre ref={textRegion} className="description check-log-text" tabIndex={0} aria-label="Log text">{lines.slice(active*200,(active+1)*200).join('\n')}</pre>{pages>1&&<div className="pagination"><button disabled={!active} onClick={()=>turn(active-1)}>Previous</button><span>Page {active+1} of {pages}</span><button disabled={active+1===pages} onClick={()=>turn(active+1)}>Next</button></div>}</>}</section>;
}
