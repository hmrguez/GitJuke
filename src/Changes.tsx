import React,{useEffect,useRef,useState} from 'react';
import {command,type ChangedFile,type ChangedFiles,type ChangesJob,type Overview,type Patch,type Snapshot} from './api';
import {type DiffModel} from './diff-model';
import './changes.css';
import {Conversation} from './Conversation';
import {AllChanges} from './AllChanges';
import {ChangesQueue} from './changes-queue';
const waiting='Queued —';
const labels:Record<Patch['state'],string>={complete:'',missing:'Changes aren’t available for this file.',binary:'This file can’t be previewed.',tooLarge:'This file is too large to preview.',incomplete:'GitHub hasn’t supplied the full changes for this file.',large:'Loading changes…'};
type ChangesProps={pr:Overview;currentHead:string;paged:boolean;revision:number;onMode:(paged:boolean)=>void;destination?:string;onBack?:()=>void;onBrowse?:()=>void};
export function Changes(props:ChangesProps){
  return props.destination?<FileChanges {...props}/>:<AllChanges {...props}/>;
}
export function FileChanges({pr,currentHead,revision,destination,onBack,onBrowse,embedded}:{pr:Overview;currentHead:string;paged:boolean;revision:number;onMode:(paged:boolean)=>void;destination?:string;onBack?:()=>void;onBrowse?:()=>void;embedded?:{file:ChangedFile;base:string;queue:ChangesQueue}}){
  const [head,setHead]=useState(pr.head),[base,setBase]=useState<string|null>(embedded?.base??null);
  const [filePage,setFilePage]=useState(1),[files,setFiles]=useState<Snapshot<ChangedFiles|null>|null>(null);
  const [file,setFile]=useState<ChangedFile|null>(embedded?.file??null);
  const [sourcePatch,setPatch]=useState<Patch|null>(null),[sourceModel,setModel]=useState<DiffModel|null>(null),[message,setMessage]=useState('Not loaded.');
  const [failed,setFailed]=useState(false);
  const navigator=useRef<HTMLElement>(null),viewport=useRef<HTMLDivElement>(null),worker=useRef<Worker|null>(null),serial=useRef(0),floor=useRef(revision),wakePatch=useRef<(()=>void)|undefined>(undefined),wakeFiles=useRef<(()=>void)|undefined>(undefined);
  floor.current=revision;
  const large=true;
  const job:ChangesJob={repo:pr.repo,number:pr.number,head,base,page:filePage,index:file?.index??null,path:file?.path??null,large,seq:0};
  const target=JSON.stringify([pr.repo,pr.number,head,base,file?.index,file?.path,large]);
  const [patchFor,setPatchFor]=useState(''),[modelFor,setModelFor]=useState('');
  // Gate at render time as well as in effects: a selection commit cannot paint old rows under a new path.
  const patch=patchFor===target?sourcePatch:null,model=modelFor===target?sourceModel:null;
  const jobRef=useRef(job);jobRef.current=job;
  const [fileError,setFileError]=useState(''),[fileRefresh,setFileRefresh]=useState(0);
  useEffect(()=>()=>{worker.current?.terminate();if(!embedded)void command('cancel_changes').catch(()=>{});},[]);
  useEffect(()=>{
    if(embedded)return;
    let active=true,busy=false;const request:ChangesJob={repo:pr.repo,number:pr.number,head,base:null,page:filePage,index:null,path:null,large:false,seq:0};
    // Page one resolves the base revision; later pages must use that same base.
    request.base=filePage===1?null:base;
    setFiles(null);setFileError('Loading file metadata…');
    async function poll(){if(busy)return;busy=true;try{
      const p=await command<Snapshot<ChangedFiles|null>>('read_files',{job:request});
      if(active&&p.revision>=floor.current){setFiles(p);setFileError(p.error??'');if(p.data){setBase(p.data.base);}}
    }catch(e){if(active)setFileError(String(e));}finally{busy=false;}}
    void (async()=>{
      try{const p=await command<Snapshot<ChangedFiles|null>>('read_files',{job:request});if(!active)return;
        if(p.data){setFiles(p);setBase(p.data.base);setFileError(p.error??'');}
        if(!p.data||p.error||fileRefresh>0){await command('request_changes',{job:request});if(active)await poll();}
      }catch(e){if(active)setFileError(String(e));}
    })();
    wakeFiles.current=()=>void poll();
    const timer=setInterval(()=>void poll(),1000);
    return()=>{active=false;wakeFiles.current=undefined;clearInterval(timer);};
    // Base is intentionally captured by page navigation, not an extra first-page refetch.
  },[pr.repo,pr.number,head,filePage,fileRefresh]);
  useEffect(()=>{
    const ticket=++serial.current;let active=true,busy=false,transferred=false,ready=false;
    let release:(()=>void)|undefined;
    const done=()=>{release?.();release=undefined;};
    worker.current?.terminate();setFailed(false);setModel(null);setPatch(null);setMessage(file?'Loading selected patch…':'Select a file to load its patch.');
    if(!file||!base)return;
    const request={...jobRef.current};
    async function poll(){if(!ready||busy||transferred)return;busy=true;try{
      const p=await command<Snapshot<Patch|null>>('read_patch',{job:request});
      if(!active||ticket!==serial.current||p.revision<floor.current)return;
      if(p.error&&(!p.data||p.error.startsWith(waiting))){setMessage(p.error);if(!p.error.startsWith(waiting)){setFailed(true);transferred=true;done();}return;}
      if(!p.data){setMessage('Queued or loading selected patch…');return;}
      setPatchFor(target);setPatch(p.data);setMessage(labels[p.data.state]+(p.error?` Cached read; latest fetch failed: ${p.error}`:''));
      if(p.data.state!=='complete'){transferred=true;done();return;}
      transferred=true;setMessage('Verifying and preparing selected patch…');const manifest=p.data;
      if(!manifest.digest||manifest.chunks>22||!manifest.bytes||!manifest.rows)throw new Error('Invalid patch transfer descriptor.');
      let source='';
      for(let chunk=0;chunk<manifest.chunks;chunk++){
        const part=await command<Snapshot<string|null>>('read_patch_chunk',{job:request,digest:manifest.digest,chunk});
        if(!active||ticket!==serial.current)return;
        if(part.revision<floor.current||part.data===null||part.error)throw new Error('Patch chunk unavailable or evicted. Reload the file.');
        source+=part.data;if(source.length>2097152)throw new Error('Patch source limit exceeded.');
      }
      if(!active)return;
      const w=new Worker(new URL('./diff-worker.ts',import.meta.url),{type:'module'});worker.current=w;
      w.onmessage=({data}:{data:{model?:DiffModel;error?:string}})=>{
        if(active&&ticket===serial.current){if(data.error){setMessage(data.error);setFailed(true);}else if(data.model){setModelFor(target);setModel(data.model);setMessage(labels.complete+(p.error?` Cached read; latest fetch failed: ${p.error}`:''));}}
        w.terminate();done();
      };
      w.onerror=()=>{if(active){setMessage('Couldn’t load changes.');setFailed(true);}w.terminate();done();};
      w.postMessage({source,bytes:manifest.bytes,rows:manifest.rows,digest:manifest.digest});source='';
    }catch(e){if(active){setMessage(String(e));setFailed(true);transferred=true;done();}}finally{busy=false;}}
    void(async()=>{try{
      let p=await command<Snapshot<Patch|null>>('read_patch',{job:request});if(!active)return;
      if(!p.data||p.error||fileRefresh>0){
        if(embedded){release=await embedded.queue.acquire();if(!active){done();return;}}
        // Another batch may have populated this patch while the slot was held.
        p=await command<Snapshot<Patch|null>>('read_patch',{job:request});if(!active){done();return;}
        if(!p.data||p.error||fileRefresh>0)await command('request_changes',{job:request});
      }
      ready=true;
      if(active)await poll();
    }catch(e){if(active){setMessage(String(e));setFailed(true);transferred=true;}done();}})();
    wakePatch.current=()=>void poll();
    const timer=setInterval(()=>void poll(),1000);
    return()=>{active=false;wakePatch.current=undefined;clearInterval(timer);worker.current?.terminate();done();};
  },[file,base,head,large,fileRefresh,pr.repo,pr.number]);
  useEffect(()=>{wakeFiles.current?.();wakePatch.current?.();},[revision]);
  // Clear/denial/account cache invalidations are authoritative even after transfer.
  useEffect(()=>{
    let active=true;if(!file||!model)return;
    void command<Snapshot<Patch|null>>('read_patch',{job:jobRef.current}).then(p=>{
      if(active&&p.revision>=floor.current){if(!p.data){setFailed(true);setModel(null);setPatch(null);setMessage(p.error??'Downloaded content was removed. Reload this file.');}else if(p.error&&!p.error.startsWith(waiting)){setMessage(`Cached patch. Latest fetch failed: ${p.error}`);}}
    }).catch(e=>{if(active){setFailed(true);setModel(null);setMessage(String(e));}});
    return()=>{active=false;};
  },[revision,file,model]);
  function choose(f:ChangedFile){
    void command('cancel_changes').catch(()=>{});setModel(null);setPatch(null);setFile(f);
  }
  function jump(row:number){
    const el=viewport.current;
    el?.querySelector(`[aria-rowindex="${row+2}"]`)?.scrollIntoView?.({block:'center'});
    el?.focus({preventScroll:true});
  }
  const stale=currentHead!==head;
  return <section id={embedded?undefined:"changes"} className={embedded?"file-patch":"changes"} role={embedded?"region":"tabpanel"} aria-label={embedded?file?.path:undefined} aria-labelledby={embedded?undefined:"changes-tab"}>
    {!embedded&&<>
    <div className="reader-actions"><button onClick={()=>{void command('cancel_changes');setFile(null);setBase(null);setHead(currentHead);setFilePage(1);setFileRefresh(n=>n+1);}}>Refresh</button></div>
    {stale&&<p className="state-banner" role="status">New changes are available. Refresh to see them.</p>}
    <nav ref={navigator} tabIndex={-1} aria-label="Changed files" className="file-navigator">

      {fileError&&files?.error&&<p className="state-banner" role="status">Couldn’t load files. Refresh to try again.</p>}
      <div className="file-list">{files?.data?.items.map(f=><button key={f.index} aria-current={file?.index===f.index?'true':undefined} onClick={()=>choose(f)}><span className="file-path">{f.path}{f.previousPath&&<small>from {f.previousPath}</small>}</span><span className="file-stat">{f.status} · +{f.additions} −{f.deletions}</span></button>)}</div>
      <div className="pagination"><button disabled={filePage===1} onClick={()=>{setFile(null);setFilePage(n=>n-1);navigator.current?.focus();}}>Previous files</button><span>Page {filePage}</span><button disabled={!files?.data?.nextPage} onClick={()=>{setFile(null);setFilePage(files?.data?.nextPage??1);navigator.current?.focus();}}>Next files</button></div>
    </nav>
    </>}
    <div className="patch-heading"><h2>{file?.path??'Select a file'}</h2>{file&&<span className="file-stat"><span className="diff-additions">+{file.additions}</span><span className="diff-deletions">−{file.deletions}</span></span>}</div>
    {!model&&<p className="reader-meta" role="status" aria-live="polite">{failed?'Couldn’t load these changes. Refresh to try again.':patch&&patch.state!=='complete'?labels[patch.state]:file?'Loading changes…':'Select a file to see its changes.'}</p>}
    {model&&message!==labels.complete&&<p className="reader-meta" role="status">Couldn’t refresh these changes. Showing the saved version.</p>}
    {model&&<div ref={viewport} className="diff-viewport" tabIndex={0} role="region" aria-label={`${file?.path} changes`}>
      <table className="diff-source">
        <caption className="sr-only">{file?.path} changes</caption>
        <thead className="sr-only"><tr><th scope="col">Old line</th><th scope="col">New line</th><th scope="col">Change and code</th></tr></thead>
        <tbody>{model.rows.map((r,i)=><tr className={`diff-row ${r.kind}`} key={i} aria-rowindex={i+2}><td className="line-number">{r.old??''}</td><td className="line-number">{r.next??''}</td><td className="code-cell">{r.kind!=='hunk'&&<span className="sr-only">{r.kind}: </span>}<code>{r.text}</code></td></tr>)}</tbody>
      </table>
    </div>}
    {!embedded&&<Conversation key={`${head}:${destination??"ordinary"}`} destination={destination} onBack={onBack} onBrowse={onBrowse} inline pr={{...pr,head}} currentHead={currentHead} revision={revision} context={model&&file&&base?{model,file,head,base,jump}:undefined}/>}
  </section>;
}
