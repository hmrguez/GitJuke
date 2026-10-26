import React,{act} from 'react';
import {createRoot} from 'react-dom/client';
import {Window} from 'happy-dom';
import {expect,test} from 'bun:test';
import type {ConversationJob} from '../../src/lib/ipc';

test('cached PR overview and checks are available while the shared conversation loads in the background',async()=>{
 const dom=new Window({url:'http://localhost'});
 Object.assign(globalThis,{window:dom,document:dom.document,HTMLElement:dom.HTMLElement,isTauri:true,IS_REACT_ACT_ENVIRONMENT:true,requestAnimationFrame:(cb:FrameRequestCallback)=>setTimeout(cb,0)});
 const head='a'.repeat(40),base='b'.repeat(40);
 const pr={id:'PR_1',repo:'a/b',number:1,title:'Ready PR',url:'https://github.com/a/b/pull/1',body:'description',bodyLimited:false,head,base,state:'OPEN',draft:false,author:'reader',stack:{state:'none'}};
 const status={settings:{alerts:{onboarded:true},paused:true,pagedChanges:false,theme:{mode:'dark'}},identity:{id:42,login:'reader'},locked:false,health:'',revision:1,selected:null,storageStopped:false,storage:{warning:false},watchCount:1,delayedWatches:0,discovery:[],budgets:[],ignoredOverrides:[],nextRemoteAt:0};
 const message=(id:string)=>({id,kind:'comment',author:'reader',time:'2026-10-26T10:00:00Z',url:null,body:id,state:'complete',bytes:id.length,digest:id});
 const page=(source:string)=>({head,source,generation:1,messages:[],threads:[],next:null,total:0,complete:true});
 const snapshot=(data:unknown)=>({data,revision:1,error:null,complete:true,fetchedAt:Math.floor(Date.now()/1000)});
 let finish!:(value:unknown)=>void;let blocked=false;let tailReady=false;let reads=0;let checkReads=0;
 (dom as any).__TAURI_INTERNALS__={invoke:async(name:string,args:any)=>{
  if(name==='runtime_status')return status;
  if(name==='saved_queries')return {items:[],nextOffset:null,revision:1};
  if(name==='authored_page'||name==='requested_page')return {items:[pr],nextOffset:null,complete:true,revision:1};
  if(name==='read_overview')return snapshot(pr);
  if(name==='read_checks'){checkReads++;return {items:[],commits:[],head,revision:1,nextOffset:null};}
  if(name==='read_files')return snapshot({head,base,total:0,items:[],nextPage:null,complete:true,limited:false});
  if(name==='read_conversation'){
   reads++;const job=args.job as ConversationJob;
   if(job.after){if(tailReady)return snapshot({...page('comments'),messages:[message('final comment')]});blocked=true;return new Promise(resolve=>{finish=value=>{tailReady=true;resolve(value);};});}
   if(job.source!=='initial')return snapshot({...page(job.source),messages:job.source==='comments'?[message('first comment')]:[],next:job.source==='comments'?'tail':null});
   return snapshot({...page('initial'),seeds:['comments','reviews','threads'].map(source=>({thread:null,page:{...page(source),messages:source==='comments'?[message('first comment')]:[],next:source==='comments'?'tail':null}}))});
  }
  return null;
 }};
 const {App}=await import('../../src/app/App');
 const host=document.createElement('div');document.body.append(host);const root=createRoot(host);
 const settle=()=>act(async()=>{await new Promise(resolve=>setTimeout(resolve,5));});
 try{
  await act(async()=>root.render(<App/>));await settle();
  await act(async()=>host.querySelector<HTMLButtonElement>('.pr-list button')!.click());await settle();
  expect(blocked).toBe(true);expect(host.querySelector('[role="tablist"]')).not.toBeNull();
  expect(host.querySelector('#overview')?.textContent).toContain('description');
  expect(host.querySelector('[aria-label="Loading pull request"]')).toBeNull();
  expect(checkReads).toBeGreaterThan(0);
  const pendingReads=reads;
  await act(async()=>host.querySelector<HTMLButtonElement>('#conversation-tab')!.click());
  expect(host.querySelector('[aria-label="Loading conversation"]')).not.toBeNull();
  expect(reads).toBe(pendingReads);
  await act(async()=>finish(snapshot({...page('comments'),messages:[message('final comment')]})));await settle();
  expect(host.querySelector('[role="tablist"]')).not.toBeNull();
  const conversation=host.querySelector('#conversation')!;
  expect(conversation.textContent).toContain('first comment');expect(conversation.textContent).toContain('final comment');
  expect(conversation.querySelector('.skeleton')).toBeNull();
  const loadedReads=reads;
  await act(async()=>host.querySelector<HTMLButtonElement>('#conversation-tab')!.click());
  expect(conversation.querySelector('.skeleton')).toBeNull();expect(reads).toBe(loadedReads);
  await act(async()=>host.querySelector<HTMLButtonElement>('#overview-tab')!.click());
  await act(async()=>host.querySelector<HTMLButtonElement>('#conversation-tab')!.click());
  expect(host.querySelector('#conversation')).toBe(conversation);expect(conversation.querySelector('.skeleton')).toBeNull();expect(reads).toBe(loadedReads);
 }finally{await act(async()=>root.unmount());host.remove();}
});
