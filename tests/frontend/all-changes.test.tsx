import React,{act} from 'react';
import {createRoot} from 'react-dom/client';
import {Window} from 'happy-dom';
import {expect,test} from 'bun:test';
import {mockIPC,clearMocks} from '@tauri-apps/api/mocks';
import {Changes} from '../../src/features/changes/Changes';
import {ChangesQueue} from '../../src/features/changes/changes-queue';
import {parseDiff} from '../../src/features/changes/diff-model';
import type {ChangedFile,ChangesJob,Overview} from '../../src/lib/ipc';

const pr:Overview={id:'pr',repo:'test/repo',number:1,title:'All files',url:'https://github.com/test/repo/pull/1',body:null,bodyLimited:false,head:'a'.repeat(40),base:'b'.repeat(40),state:'OPEN',draft:false,author:null,stack:{state:'none'}};
const files:ChangedFile[]=Array.from({length:21},(_,index)=>({index,path:`file-${index}.ts`,previousPath:null,status:'modified',additions:1,deletions:1,url:null}));
const snapshot=(data:unknown)=>({data,error:null,revision:100,fetchedAt:1,complete:true});
const source='@@ -1 +1 @@\n-before\n+after\n';
const flush=()=>act(async()=>{await new Promise(resolve=>setTimeout(resolve,0));});

test('ChangesQueue holds each native slot until its reader finishes',async()=>{
  const queue=new ChangesQueue(),release=await queue.acquire();let second=false;
  const pending=queue.acquire().then(done=>{second=true;done();});
  await Promise.resolve();expect(second).toBe(false);release();await pending;expect(second).toBe(true);
});

test('all-file review walks metadata pages, eagerly loads every patch without viewport events and navigates to distant files',async()=>{
  const dom=new Window();
  const observers=new Map<Element,(near:boolean)=>void>();
  class Observer {
    constructor(private callback:(entries:{isIntersecting:boolean}[])=>void){}
    node:Element|null=null;
    observe(node:Element){this.node=node;observers.set(node,near=>this.callback([{isIntersecting:near}]));}
    disconnect(){if(this.node)observers.delete(this.node);}
  }
  class WorkerStub {
    onmessage:((event:{data:unknown})=>void)|null=null;
    postMessage({source}:{source:string}){queueMicrotask(()=>this.onmessage?.({data:{model:parseDiff(source)}}));}
    terminate(){}
  }
  Object.assign(globalThis,{window:dom,document:dom.document,isTauri:true,IS_REACT_ACT_ENVIRONMENT:true,IntersectionObserver:Observer,ResizeObserver:class{observe(){}disconnect(){}},Worker:WorkerStub});
  dom.HTMLElement.prototype.getBoundingClientRect=()=>({height:24}) as DOMRect;
  dom.HTMLElement.prototype.scrollIntoView=function(){};
  let secondReady=false,pageRequested=false;
  const pages:number[]=[],requests:number[]=[],available=new Set(files.map(f=>f.index));let cancellations=0,cleared=false;
  mockIPC((name,args)=>{
    const job=args?.job as ChangesJob;
    if(name==='read_files'){
      pages.push(job.page);
      if(cleared)return snapshot(null);
      if(job.page===2){expect(job.base).toBe(pr.base);return secondReady?snapshot({head:pr.head,base:pr.base,total:21,items:files.slice(20),nextPage:null,complete:false,limited:false}):{...snapshot(null),error:pageRequested?'Queued — loading':null};}
      return snapshot({head:pr.head,base:pr.base,total:21,items:job.page===1?files.slice(0,20):files.slice(20),nextPage:job.page===1?2:null,complete:job.page===2,limited:false});
    }
    if(name==='request_changes'){if(job.index===null){pageRequested=true;return;}requests.push(job.index!);available.add(job.index!);return;}
    if(name==='read_patch'){expect(job.large).toBe(true);return snapshot(available.has(job.index!)?{head:pr.head,base:pr.base,file:files[job.index!],state:'complete',bytes:source.length,rows:3,chunks:1,digest:'test'}:null);}
    if(name==='read_patch_chunk')return snapshot(source);
    if(name==='cancel_changes'){cancellations++;return;}
    if(name==='read_conversation')return snapshot({head:pr.head,generation:1,source:'threads',messages:[],threads:[],next:null,total:0,complete:true});
    return;
  });
  const host=document.createElement('div');document.body.append(host);const root=createRoot(host);
  try{
    await act(async()=>root.render(<Changes pr={pr} currentHead={pr.head} paged={false} revision={1} onMode={()=>{}}/>));
    await flush();
    expect(pages).toContain(2);expect(pageRequested).toBe(true);
    expect(host.querySelectorAll('.all-file-section').length).toBe(20);
    expect(host.querySelector<HTMLButtonElement>('.file-list button')!.disabled).toBe(false);
    const first=host.querySelector('.all-file-section')!;
    await flush();
    expect(first.querySelector('table')).not.toBeNull();
    for(const label of ['Hunk','Reload file','Reading mode','UTF-8','Head','files loaded','Single file'])expect(host.textContent).not.toContain(label);
    expect(first.querySelectorAll('button')).toHaveLength(0);
    expect(first.querySelectorAll('.diff-row')).toHaveLength(3);
    expect(requests).toEqual([]);
    secondReady=true;
    await act(async()=>root.render(<Changes pr={pr} currentHead={pr.head} paged={false} revision={2} onMode={()=>{}}/>));await flush();
    expect(host.querySelectorAll('.all-file-section').length).toBe(21);
    expect(host.textContent).toContain('21 files');expect(requests).toEqual([]);
    const sections=host.querySelectorAll<HTMLElement>('.all-file-section');
    await flush();
    expect(requests).toEqual([]);expect(host.querySelectorAll('table.diff-source').length).toBe(21);
    expect(host.textContent).toContain('+after');expect(cancellations).toBe(0);
    // Even the final file is ready before its navigator link is activated.
    await act(async()=>{host.querySelectorAll<HTMLButtonElement>('.file-list button')[20].click();});
    await flush();expect(requests).toEqual([]);expect(sections[20].querySelector('table')).not.toBeNull();
    // Keep loaded changes in the document when scrolling away.
    expect(sections[0].querySelector('table')).not.toBeNull();expect(sections[0].textContent).toContain('file-0.ts');
    // Authoritative cache removal hides every retained reader.
    cleared=true;
    await act(async()=>root.render(<Changes pr={pr} currentHead={pr.head} paged={false} revision={101} onMode={()=>{}}/>));
    await flush();expect(host.querySelectorAll('table.diff-source').length).toBe(0);expect(host.textContent).toContain('Couldn’t load changes. Refresh to try again.');
  }finally{await act(async()=>root.unmount());host.remove();clearMocks();Object.assign(globalThis,{isTauri:false});}
});

test('clearing progressive results rejects a late metadata batch',async()=>{
  const dom=new Window();
  Object.assign(globalThis,{window:dom,document:dom.document,isTauri:true,IS_REACT_ACT_ENVIRONMENT:true,IntersectionObserver:class{observe(){}disconnect(){}}});
  let cleared=false,finish!:(data:unknown)=>void;
  const pending=new Promise(resolve=>{finish=resolve;});
  const data=(page:number)=>({head:pr.head,base:pr.base,total:21,items:page===1?files.slice(0,20):files.slice(20),nextPage:page===1?2:null,complete:false,limited:false});
  mockIPC((name,args)=>{
    if(name==='read_files'){
      const page=(args?.job as ChangesJob).page;
      return cleared?snapshot(null):page===1?snapshot(data(1)):pending;
    }
    if(name==='read_conversation')return snapshot({head:pr.head,generation:1,source:'threads',messages:[],threads:[],next:null,total:0,complete:true});
  });
  const host=document.createElement('div');document.body.append(host);const root=createRoot(host);
  try{
    await act(async()=>root.render(<Changes pr={pr} currentHead={pr.head} paged={false} revision={1} onMode={()=>{}}/>));await flush();
    expect(host.querySelectorAll('.all-file-section')).toHaveLength(20);
    cleared=true;
    await act(async()=>root.render(<Changes pr={pr} currentHead={pr.head} paged={false} revision={101} onMode={()=>{}}/>));await flush();
    expect(host.querySelectorAll('.all-file-section')).toHaveLength(0);
    await act(async()=>finish(snapshot(data(2))));await flush();
    expect(host.querySelectorAll('.all-file-section')).toHaveLength(0);
    expect(host.textContent).toContain('Couldn’t load changes. Refresh to try again.');
  }finally{finish(snapshot(null));await act(async()=>root.unmount());host.remove();clearMocks();Object.assign(globalThis,{isTauri:false});}
});
