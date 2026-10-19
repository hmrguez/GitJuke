import {Skeleton} from '../../components/Skeleton';
import React,{useRef} from 'react';
import {Check,CircleX,CircleDot} from 'lucide-react';
import type {CheckContext,CheckPage} from '../../lib/ipc';
import {CheckLog} from './CheckLog';

const results:Record<string,string>={success:'Passed',failure:'Failed',timed_out:'Timed out',action_required:'Action needed',error:'Failed',stale:'Needs rerun',pending:'Pending',queued:'Queued',in_progress:'Running',waiting:'Waiting',requested:'Requested',expected:'Expected',cancelled:'Cancelled',skipped:'Skipped',neutral:'Neutral',completed:'Completed'};
const reviews:Record<string,string>={APPROVED:'Approved',CHANGES_REQUESTED:'Changes requested',REVIEW_REQUIRED:'Review required'};
function CheckResult({check}:{check:CheckContext}){
  const result=(check.conclusion??check.status).toLowerCase();
  const tone=result==='success'?'success':['failure','timed_out','action_required','error','stale'].includes(result)?'failure':['pending','queued','in_progress','waiting','requested','expected'].includes(result)?'pending':'neutral';
  return <span className={`check-result ${tone}`}>{tone==='success'?<Check size={14}/>:tone==='failure'?<CircleX size={14}/>:<CircleDot size={14}/>} {results[result]??'Status unavailable'}</span>;
}
export function CheckReader({repo,number,checks,offset,setOffset,revision}:{revision:number;repo:string;number:number;checks:CheckPage|null;offset:number;setOffset:(n:number)=>void}){
  const region=useRef<HTMLElement>(null);
  const complete=!!checks?.commits.length&&checks.commits.every(c=>c.complete&&!c.error);
  const review=checks?.review?reviews[checks.review]:null;
  return <section ref={region} tabIndex={0} id="checks" className="overview checks" role="tabpanel" aria-labelledby="checks-tab">
    {review&&<div className="reader-actions"><span className={`review-result ${checks?.review==='APPROVED'?'success':checks?.review==='CHANGES_REQUESTED'?'failure':'pending'}`}>{review}</span></div>}
    {(checks?.error||checks?.commits.some(c=>c.error))&&<p className="state-banner" role="status">Couldn’t refresh checks. Results may be out of date.</p>}
    {!checks&&<Skeleton shape="checks" label="Loading checks"/>}
    {checks&&!checks.items.length&&<p className="empty">{complete?'No checks to show.':'Checks aren’t available yet.'}</p>}
    {checks?.items.map(c=><div className="check-row" key={`${c.commit}:${c.id}`}><strong>{c.name}</strong><CheckResult check={c}/><CheckLog key={JSON.stringify([repo,number,checks.head,c,checks.error,checks.commits.map(s=>[s.resource,s.complete,s.error])])} job={{repo,number,head:checks.head??'',check:c.id,offset}} check={c} revision={revision}/></div>)}
    {(offset>0||checks?.nextOffset!=null)&&<div className="pagination"><button disabled={offset===0} onClick={()=>{setOffset(Math.max(0,offset-100));region.current?.focus();}}>Previous checks</button><span>Page {Math.floor(offset/100)+1}</span><button disabled={checks?.nextOffset==null} onClick={()=>{setOffset(checks?.nextOffset??0);region.current?.focus();}}>Next checks</button></div>}
  </section>;
}
