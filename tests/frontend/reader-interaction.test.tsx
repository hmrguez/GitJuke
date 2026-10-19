import React,{act} from 'react';
import {createRoot,type Root} from 'react-dom/client';
import {Window} from 'happy-dom';
import {afterEach,beforeEach,expect,test} from 'bun:test';
import {Changes,FileChanges} from '../../src/features/changes/Changes';
import {parseDiff} from '../../src/features/changes/diff-model';
import {Conversation,Message} from '../../src/features/conversation/Conversation';
import type {ConversationJob,ConversationPage,ConversationMessage,ReviewThread} from '../../src/lib/ipc';

const dom=new Window({url:'http://localhost'});
Object.assign(globalThis,{window:dom,document:dom.document,HTMLElement:dom.HTMLElement,IS_REACT_ACT_ENVIRONMENT:true,isTauri:true,requestAnimationFrame:(cb:FrameRequestCallback)=>setTimeout(cb,0)});
const job:ConversationJob={repo:'a/b',number:1,head:'a'.repeat(40),source:'destination',thread:'target',after:null,seq:0};
const message:ConversationMessage={id:'target',kind:'comment',author:'reader',time:'now',url:null,body:'a'.repeat(70000),state:'complete',bytes:70000,digest:'digest'};
const pr={id:'pr',repo:'a/b',number:1,head:job.head,base:'main',title:'Reader',url:'',body:null,bodyLimited:false,state:'open',draft:false,author:null,stack:{state:'none' as const}};
const thread:ReviewThread={id:'T',path:'one.ts',side:'RIGHT',line:1,startLine:null,startSide:null,originalLine:null,originalStartLine:null,commit:job.head,originalCommit:null,outdated:false,resolved:false,url:null,time:'now'};
let readRevision=1;
let root:Root,host:HTMLElement,calls:{name:string;args:any}[],read:(j:ConversationJob)=>Promise<ConversationPage|null>;
const page=(j:ConversationJob):ConversationPage=>{
 const data:ConversationPage={head:j.head,source:j.source,messages:j.source==='threads'||j.source==='initial'?[]:[{...message,body:j.after?'second page':'first page',bytes:10}],threads:j.source==='threads'?[thread]:[],generation:1,next:j.after||j.source==='initial'?null:'cursor',total:2,complete:!!j.after,destinationAnchor:null};
 if(j.source==='initial')data.seeds=(['comments','reviews','threads'] as const).map(source=>({thread:null,page:page({...j,source})}));
 if(j.source==='threads')data.seeds=[{thread:thread.id,page:page({...j,source:'replies',thread:thread.id,after:null})}];
 return data;
};
const settle=()=>act(async()=>{await new Promise(resolve=>setTimeout(resolve,0));});
const button=(name:string)=>Array.from(host.querySelectorAll('button')).find(b=>b.textContent===name)!;
async function click(name:string){const el=button(name);expect(el).toBeDefined();await act(async()=>{el.focus();el.click();});await settle();}
async function render(node:React.ReactNode){await act(async()=>root.render(node));await settle();}
beforeEach(()=>{Object.assign(globalThis,{IntersectionObserver:class{constructor(private callback:any){}observe(){this.callback([{isIntersecting:true}]);}disconnect(){}}});host=document.createElement('div');document.body.append(host);root=createRoot(host);calls=[];readRevision=1;read=async j=>page(j);(dom as any).__TAURI_INTERNALS__={invoke:async(name:string,args:any)=>{calls.push({name,args});if(name==='read_conversation')return {data:await read(args.job),revision:readRevision,error:null};return null;}};});
afterEach(async()=>{await act(async()=>root.unmount());host.remove();});

test('ordinary segment navigation focuses the scrollable new segment and resets its scroll',async()=>{
 await render(<Message message={message} job={job} generation={1} report={()=>{}}/>);
 const body=host.querySelector<HTMLElement>('.message-body')!;expect(body.tabIndex).toBe(0);body.scrollTop=300;
 await click('Next part');expect(document.activeElement===body).toBe(true);expect(body.scrollTop).toBe(0);expect(body.getAttribute('aria-label')).toContain('segment 2 of 3');
 expect(calls.filter(c=>c.name==='conversation_seen')).toHaveLength(0);
});
test('exact destination counts committed distinct segments without stealing focus on polling',async()=>{
 const node=<Message message={message} job={job} generation={1} explicitlyOpened report={()=>{}}/>;await render(node);
 expect(document.activeElement===host.querySelector('article')).toBe(true);expect(calls.filter(c=>c.name==='conversation_seen')).toHaveLength(0);
 await click('Next part');await click('Previous part');await click('Next part');expect(calls.filter(c=>c.name==='conversation_seen')).toHaveLength(0);
 await click('Next part');expect(document.activeElement===host.querySelector('.message-body')).toBe(true);expect(calls.filter(c=>c.name==='conversation_seen')).toHaveLength(1);
 const other=document.createElement('button');document.body.append(other);other.focus();await render(node);expect(document.activeElement===other).toBe(true);other.remove();
});
test('oversized destination remains unseen; complete empty review submits its exact receipt',async()=>{
 await render(<Message key="limited" message={{...message,body:null,state:'oversized'}} job={job} generation={1} explicitlyOpened report={()=>{}}/>);expect(calls.filter(c=>c.name==='conversation_seen')).toHaveLength(0);
 await render(<Message key="empty" message={{...message,kind:'APPROVED',body:'',bytes:0}} job={job} generation={2} explicitlyOpened report={()=>{}}/>);
 expect(calls.filter(c=>c.name==='conversation_seen').map(c=>c.args)).toEqual([{job,id:message.id,digest:message.digest,generation:2}]);
});
test('changed destination generation restarts segment coverage; stale head never submits seen',async()=>{
 let generation=1;read=async j=>({...page(j),messages:[message],generation,next:null,total:1});
 await render(<Conversation pr={pr} revision={1} currentHead={pr.head} destination="target"/>);await click('Next part');
 generation=2;readRevision=2;await render(<Conversation pr={pr} revision={2} currentHead={pr.head} destination="target"/>);await click('Next part');await click('Next part');
 expect(calls.filter(c=>c.name==='conversation_seen')).toHaveLength(1);expect(calls.find(c=>c.name==='conversation_seen')!.args.generation).toBe(2);
 await render(<Conversation key="stale" pr={pr} revision={1} currentHead={'b'.repeat(40)} destination="target"/>);await click('Next part');await click('Next part');
 expect(calls.filter(c=>c.name==='conversation_seen')).toHaveLength(1);
});

test('changes render every row regardless of saved paged mode; file pages focus the navigator',async()=>{
 const source='@@ -1,201 +1,201 @@\n'+Array.from({length:201},(_,i)=>` line ${i}`).join('\n');
 const model=parseDiff(source),file={index:1,path:'one.ts',previousPath:null,status:'modified',additions:0,deletions:0,url:null};
 const priorWorker=globalThis.Worker,priorObserver=globalThis.ResizeObserver;
 class TestWorker{onmessage:any;onerror:any;postMessage(){queueMicrotask(()=>this.onmessage?.({data:{model}}));}terminate(){}}
 class TestObserver{observe(){}disconnect(){}}
 Object.assign(globalThis,{Worker:TestWorker,ResizeObserver:TestObserver});
 const invoke=(dom as any).__TAURI_INTERNALS__.invoke;
 (dom as any).__TAURI_INTERNALS__.invoke=async(name:string,args:any)=>{
  const data=name==='read_files'?{head:job.head,base:'b'.repeat(40),total:21,items:[file],nextPage:args.job.page===1?2:null,complete:false,limited:false}:name==='read_patch'?{head:job.head,base:'b'.repeat(40),file,state:'complete',bytes:source.length,rows:model.rows.length,chunks:1,digest:'digest'}:name==='read_patch_chunk'?source:undefined;
  return data===undefined?invoke(name,args):{data,revision:1,error:null};
 };
 try{
  await render(<FileChanges pr={pr} currentHead={pr.head} paged revision={1} onMode={()=>{}}/>);
  await act(async()=>host.querySelector<HTMLButtonElement>('.file-list button')!.click());await settle();await settle();
  expect(button('Next rows')).toBeUndefined();expect(button('Previous rows')).toBeUndefined();expect(host.querySelectorAll('.diff-row')).toHaveLength(202);
  expect(host.querySelector('input[name=hunk]')).toBeNull();expect(host.textContent).not.toContain('UTF-8');expect(host.textContent).not.toContain('supplied change hunks');
  await click('Next files');expect(document.activeElement===host.querySelector('.file-navigator')).toBe(true);expect(button('Next files').disabled).toBe(true);expect(host.querySelector('.diff-viewport')).toBeNull();
 }finally{await render(null);Object.assign(globalThis,{Worker:priorWorker,ResizeObserver:priorObserver});}
});

test('an exact inline thread jump retains diff focus rather than overriding it with discussion focus',async()=>{
 const target=document.createElement('div');target.tabIndex=0;document.body.append(target);
 const context={head:pr.head,base:'b'.repeat(40),file:{index:1,path:thread.path,previousPath:null,status:'modified',additions:0,deletions:0,url:null},model:parseDiff('@@ -1 +1 @@\n one'),jump:()=>target.focus()};
 try{
  await render(<Conversation inline pr={pr} revision={1} currentHead={pr.head} context={context}/>);await click('Go to thread · line 1');
  expect(document.activeElement===target).toBe(true);expect(host.querySelector('.timeline-thread')).not.toBeNull();
  expect(calls.filter(c=>c.name==='conversation_seen')).toHaveLength(0);
 }finally{target.remove();}
});

test('segmented exact destinations retain traversal across current-anchor patch loading and jump to a later line in the continuous diff',async()=>{
 const source='@@ -0,0 +1,201 @@\n'+Array.from({length:201},(_,i)=>`+local line ${i+1}`).join('\n');
 const model=parseDiff(source),file={index:0,path:'file-0.txt',previousPath:null,status:'added',additions:201,deletions:0,url:null};
 const coordinates={...thread,id:message.id,path:file.path,line:201,startLine:199,startSide:'RIGHT'};
 read=async j=>({...page(j),messages:[message],threads:[],next:null,total:1,complete:false,destinationAnchor:coordinates});
 const priorWorker=globalThis.Worker,priorObserver=globalThis.ResizeObserver;
 class TestWorker{onmessage:any;onerror:any;postMessage(){queueMicrotask(()=>this.onmessage?.({data:{model}}));}terminate(){}}
 class TestObserver{observe(){}disconnect(){}}
 Object.assign(globalThis,{Worker:TestWorker,ResizeObserver:TestObserver});
 const invoke=(dom as any).__TAURI_INTERNALS__.invoke;
 (dom as any).__TAURI_INTERNALS__.invoke=async(name:string,args:any)=>{
  const data=name==='read_files'?{head:job.head,base:'b'.repeat(40),total:1,items:[file],nextPage:null,complete:true,limited:false}:name==='read_patch'?{head:job.head,base:'b'.repeat(40),file,state:'complete',bytes:source.length,rows:model.rows.length,chunks:1,digest:'patch-digest'}:name==='read_patch_chunk'?source:undefined;
  return data===undefined?invoke(name,args):{data,revision:1,error:null};
 };
 try{
  await render(<Changes pr={pr} currentHead={pr.head} paged revision={1} destination="target" onMode={()=>{}}/>);
  expect(button('Return to diff · line 201')).toBeUndefined();await click('Next part');
  await act(async()=>host.querySelector<HTMLButtonElement>('.file-list button')!.click());await settle();await settle();
  expect(host.textContent).toContain('line 201');await click('Return to diff · line 201');
  expect(document.activeElement===host.querySelector('.diff-viewport')).toBe(true);expect(host.querySelectorAll('.diff-row')).toHaveLength(202);expect(host.querySelector('.diff-source')!.textContent).toContain('local line 201');
  expect(calls.filter(c=>c.name==='conversation_seen')).toHaveLength(0);await click('Next part');
  expect(calls.filter(c=>c.name==='conversation_seen')).toHaveLength(1);expect(calls.find(c=>c.name==='conversation_seen')!.args.id).toBe('target');
 }finally{await render(null);Object.assign(globalThis,{Worker:priorWorker,ResizeObserver:priorObserver});}
});

test('leaving a partial exact destination releases authorization and remounts fresh segment coverage',async()=>{
 read=async j=>j.source==='initial'?page(j):({...page(j),messages:[message],threads:[],next:null,total:1});
 const props={pr,revision:1,currentHead:pr.head};
 await render(<Conversation key="opened" {...props} destination="target"/>);await click('Next part');
 await render(<Conversation key="ordinary" {...props}/>);expect(button('Next part')).toBeUndefined();
 expect(calls.filter(c=>c.name==='conversation_seen')).toHaveLength(0);
 await render(<Conversation key="reopened" {...props} destination="target"/>);expect(host.textContent).toContain('Part 1 of 3');
 await click('Next part');expect(calls.filter(c=>c.name==='conversation_seen')).toHaveLength(0);
 await click('Next part');expect(calls.filter(c=>c.name==='conversation_seen')).toHaveLength(1);
});

