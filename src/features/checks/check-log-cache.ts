import {useEffect,useRef,useState} from 'react';
import {command,type CheckContext,type CheckPage} from '../../lib/ipc';
import type {LogJob,LogRead} from './CheckLog';

export function hasActionsLog(repo:string,check:CheckContext){
 const prefix=`https://github.com/${repo}/actions/runs/${check.runId}/job/`;
 const id=check.url?.startsWith(prefix)?check.url.slice(prefix.length):'';
 return check.kind==='CheckRun'&&!!check.runId&&!!check.attempt&&/^[0-9]+$/.test(id);
}
export const logKey=(check:CheckContext)=>JSON.stringify(check);

// The native transport has one log slot. Drain it serially and retain results
// for the selected PR, independently of which log the user chooses to display.
export function useCheckLogs(repo:string,number:number,checks:CheckPage|null,offset:number){
 const scope=JSON.stringify([repo,number,checks?.head,offset]);
 const identity=JSON.stringify([scope,checks?.items,checks?.error,checks?.commits.map(c=>[c.resource,c.complete,c.error])]);
 const cache=useRef<{scope:string;values:Map<string,LogRead>}>({scope,values:new Map()});
 const [saved,setSaved]=useState<{identity:string;values:Map<string,LogRead>}>({identity:'',values:new Map()});
 useEffect(()=>{
  if(cache.current.scope!==scope||!checks?.head||checks.error)cache.current={scope,values:new Map()};
  const values=cache.current.values;
  const currentKeys=new Set(checks?.items.filter(check=>checks.commits.some(c=>c.resource===check.commit&&c.complete&&!c.error)).map(logKey));
  for(const key of values.keys())if(!currentKeys.has(key))values.delete(key);
  setSaved({identity,values:new Map(values)});
  if(!checks?.head||checks.error)return;
  let disposed=false,seq:number|undefined,timer:ReturnType<typeof setTimeout>|undefined,wake:(()=>void)|undefined;
  async function load(){
   for(const [index,check] of checks!.items.entries()){
    if(disposed)return;
    if(values.has(logKey(check))||!hasActionsLog(repo,check)||!checks!.commits.some(c=>c.resource===check.commit&&c.complete&&!c.error))continue;
    const job:LogJob={repo,number,head:checks!.head!,check:check.id,offset:offset+Math.floor(index/100)*100};
    let value:LogRead={state:'failed',text:null};
    try{
     const ticket=await command<number>('request_logs',{job});seq=ticket;
     if(disposed){await command('cancel_logs',{seq:ticket});return;}
     if(typeof ticket!=='number')throw new Error('Unavailable log request');
     do{
      value=await command<LogRead>('read_logs',{seq:ticket});
      if(disposed)return;
      if(value.state!=='queued'&&value.state!=='reading')break;
      await new Promise<void>(resolve=>{wake=resolve;timer=setTimeout(resolve,500);});
     }while(!disposed);
    }catch{value={state:'failed',text:null};}
    if(seq!==undefined){await command('cancel_logs',{seq}).catch(()=>{});seq=undefined;}
    if(disposed)return;
    values.set(logKey(check),value);
    setSaved({identity,values:new Map(values)});
   }
  }
  void load();
  return()=>{disposed=true;clearTimeout(timer);wake?.();if(seq!==undefined)void command('cancel_logs',{seq}).catch(()=>{});};
 },[identity]);
 return saved.identity===identity?saved.values:new Map<string,LogRead>();
}
