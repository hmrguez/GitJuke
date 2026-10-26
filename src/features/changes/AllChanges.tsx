import {Skeleton} from '../../components/Skeleton';
import React,{useEffect,useRef,useState} from 'react';
import {command,type ChangedFile,type ChangedFiles,type ChangesJob,type Overview,type Snapshot} from '../../lib/ipc';
import {FileChanges} from './Changes';
import {ChangesQueue} from './changes-queue';

type Props={pr:Overview;currentHead:string;paged:boolean;revision:number;onMode:(paged:boolean)=>void};
export function AllChanges(props:Props){
  const [head,setHead]=useState(props.pr.head),[retry,setRetry]=useState(0);
  const {pr}=props;
  return <AllRevision key={`${pr.repo}:${pr.number}:${head}:${retry}`} {...props} refresh={retry>0} pr={{...pr,head}} update={()=>{setHead(props.currentHead);setRetry(n=>n+1);}}/>;
}
function AllRevision({pr,currentHead,paged,revision,onMode,update,refresh}:Props&{update:()=>void;refresh:boolean}){
  const [files,setFiles]=useState<ChangedFile[]>([]),[base,setBase]=useState<string|null>(null),[ready,setReady]=useState(false),[limited,setLimited]=useState(false),[error,setError]=useState('');
  const queue=useRef(new ChangesQueue()).current;
  const sections=useRef(new Map<number,HTMLElement>());
  const wakeCache=useRef<(()=>void)|undefined>(undefined),stopLoading=useRef<(()=>void)|undefined>(undefined);
  useEffect(()=>{wakeCache.current?.();},[revision]);
  const request:ChangesJob={repo:pr.repo,number:pr.number,head:pr.head,base:null,page:1,index:null,path:null,large:false,seq:0};
  useEffect(()=>{
    let active=true,timer:ReturnType<typeof setTimeout>|undefined,wake:(()=>void)|undefined;
    stopLoading.current=()=>{active=false;clearTimeout(timer);wake?.();};
    async function load(){
      try{
        let page:number|null=1,comparison:string|null=null;
        const items:ChangedFile[]=[];
        while(active&&page!==null){
          const job:ChangesJob={...request,base:comparison,page};
          let snapshot:Snapshot<ChangedFiles|null>=await command<Snapshot<ChangedFiles|null>>('read_files',{job});
          if(!active)return;
          if(!snapshot.data||snapshot.error||refresh){
            const release=await queue.acquire();
            if(!active){release();return;}
            try{
              // Cold file reads share the native slot; cache hits never wait for it.
              await command('request_changes',{job});
              do{
                snapshot=await command<Snapshot<ChangedFiles|null>>('read_files',{job});
                if(snapshot.error&&!snapshot.error.startsWith('Queued —'))throw new Error(snapshot.error);
                if(!snapshot.data||snapshot.error?.startsWith('Queued —')){
                  await new Promise<void>(resolve=>{wake=resolve;wakeCache.current=resolve;timer=setTimeout(resolve,500);});
                  clearTimeout(timer);wakeCache.current=undefined;
                }
                if(!active)return;
              }while(active&&(!snapshot.data||snapshot.error?.startsWith('Queued —')));
            }finally{release();}
          }
          if(!active)return;
          const data:ChangedFiles|null=snapshot.data;
          if(!data)throw new Error('File metadata unavailable. Reload Changes to retry.');
          if(data.head!==pr.head||(comparison!==null&&data.base!==comparison))throw new Error('The comparison changed. Update Changes to read the new revision.');
          if(data.nextPage!==null&&data.nextPage<=page)throw new Error('Invalid file pagination. Reload Changes to retry.');
          comparison=data.base;
          items.push(...data.items);
          setFiles([...items]);setBase(comparison);setLimited(data.limited);
          page=data.nextPage;
        }
        if(active)setReady(true);
      }catch(e){if(active)setError(String(e));}
    }
    void load();
    return()=>{active=false;clearTimeout(timer);wakeCache.current=undefined;wake?.();void command('cancel_changes').catch(()=>{});};
  },[pr.repo,pr.number,pr.head]);
  // Clearing downloads, losing access or changing account also hides retained readers.
  useEffect(()=>{
    if(!files.length)return;
    let active=true;
    void command<Snapshot<ChangedFiles|null>>('read_files',{job:request}).then(snapshot=>{
      if(active&&!snapshot.data){stopLoading.current?.();setReady(false);setFiles([]);setError(snapshot.error??'Downloaded Changes were removed. Reload Changes to retry.');}
    }).catch(e=>{if(active){stopLoading.current?.();setReady(false);setFiles([]);setError(String(e));}});
    return()=>{active=false;};
  },[revision,files.length]);
  function jump(index:number){const section=sections.current.get(index);section?.scrollIntoView({block:'start'});section?.focus({preventScroll:true});}
  return <section id="changes" className="changes all-changes" role="tabpanel" aria-labelledby="changes-tab">
    <div className="reader-actions">{ready&&<span className="changed-file-count">{files.length} {files.length===1?'file':'files'}</span>}<div className="changes-actions"><button onClick={update}>Refresh</button><button onClick={()=>void command('open_changes',{job:request}).catch(e=>setError(String(e)))}>View on GitHub</button></div></div>
    {currentHead!==pr.head&&<p className="state-banner" role="status">New changes are available. Refresh to see them.</p>}
    {!ready&&!error&&<><Skeleton shape="files" label="Loading changed files"/><Skeleton shape="diff" label="Loading changes"/></>}
    {limited&&<p className="reader-meta">Some files are only available on GitHub.</p>}
    {error&&<p className="state-banner" role="alert">Couldn’t load changes. Refresh to try again.</p>}
    <details className="file-navigator"><summary>Jump to file</summary><nav aria-label="Changed files" className="file-list">{files.map(file=><button key={file.index} onClick={()=>jump(file.index)}><span className="file-path">{file.path}</span><span className="file-stat"><span className="diff-additions">+{file.additions}</span><span className="diff-deletions">−{file.deletions}</span></span></button>)}</nav></details>
    {base&&files.map(file=><LoadedPatch key={file.index} {...{file,base,queue,pr,currentHead,paged,revision,onMode}} register={element=>{if(element)sections.current.set(file.index,element);else sections.current.delete(file.index);}}/>)}
    {ready&&!files.length&&<p className="empty">No changed files in this revision.</p>}
  </section>;
}
function LoadedPatch({file,base,queue,register,...props}:Props&{file:ChangedFile;base:string;queue:ChangesQueue;register:(element:HTMLElement|null)=>void}){
  return <article ref={register} tabIndex={-1} className="all-file-section" aria-label={`Changes in ${file.path}`}>
    <FileChanges {...props} embedded={{file,base,queue}}/>
  </article>;
}
