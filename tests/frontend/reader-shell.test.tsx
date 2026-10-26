import React,{act} from 'react';
import {createRoot} from 'react-dom/client';
import {Window} from 'happy-dom';
import {expect,test} from 'bun:test';

test('Browse releases receipt authority but compact Back restores the exact Activity origin even without mouse focus',async()=>{
 const dom=new Window({url:'http://localhost'});
 Object.assign(globalThis,{window:dom,document:dom.document,HTMLElement:dom.HTMLElement,IS_REACT_ACT_ENVIRONMENT:true,isTauri:true,requestAnimationFrame:(cb:FrameRequestCallback)=>setTimeout(cb,0)});
 (dom as any).matchMedia=()=>({matches:true});
 const head='a'.repeat(40),id='github.com:a/b:1:target';
 const pr={id:'PR_1',repo:'a/b',number:1,title:'Reader',url:'',body:'description',bodyLimited:false,head,base:'main',state:'open',draft:false,author:'reader',stack:{state:'none'}};
 const record={id,repo:'a/b',number:1,kind:'comment',occurred:1,actor:'reader',url:'',text:null,eligible:false,reason:'local test',seen:false,head};
 const status={settings:{alerts:{onboarded:true,enabled:false,previews:false,categories:[true,true,true,true,true],permission:'notDetermined'},paused:true,pagedChanges:true,theme:{mode:'dark'}},identity:{id:42,login:'reader'},locked:false,health:'',revision:1,selected:null,storageStopped:false,storage:{warning:false},watchCount:1,delayedWatches:0,discovery:[],budgets:[],ignoredOverrides:[],nextRemoteAt:0};
 (dom as any).__TAURI_INTERNALS__={invoke:async(name:string,args:any)=>{
  if(name==='runtime_status')return status;
  if(name==='saved_queries')return {items:[],nextOffset:null,revision:1};
  if(name==='authored_page'||name==='requested_page')return {items:[pr],nextOffset:null,complete:true,revision:1};
  if(name==='activity_page')return {items:[record],nextOffset:null,revision:1,baseline:true,pending:0};
  if(name==='read_overview')return {data:pr,fetchedAt:1,complete:true,error:null,revision:1};
  if(name==='read_checks')return {items:[],commits:[],head,revision:1};
  if(name==='read_conversation'){
   const page=(source:string)=>({head,source,generation:1,messages:source==='comments'?[{id:'target',kind:'comment',author:'reader',time:'now',url:null,body:'body',state:'complete',bytes:4,digest:'digest'}]:[],threads:[],next:null,total:1,complete:true});
   return {data:args.job.source==='initial'?{...page('initial'),seeds:['comments','reviews','threads'].map(source=>({thread:null,page:page(source)}))}:page(args.job.source),fetchedAt:Math.floor(Date.now()/1000),revision:1,error:null};
  }
  return null;
 }};
 const {App}=await import('../../src/app/App');
 const host=document.createElement('div');document.body.append(host);const root=createRoot(host);
 const settle=()=>act(async()=>{await new Promise(r=>setTimeout(r,10));});
 const click=async(selector:string)=>{await act(async()=>host.querySelector<HTMLButtonElement>(selector)!.click());await settle();};
 try{
  await act(async()=>root.render(<App/>));await settle();
  await click('nav button[title="Activity"]');
  const origin=document.getElementById(`activity-${id}`)!;expect(origin).not.toBeNull();
  // Safari/AX activation can invoke a button while another element retains focus.
  host.querySelector<HTMLElement>('nav button[title="Activity"]')!.focus();
  await act(async()=>origin.click());await settle();
  expect(host.textContent).toContain('Browse conversation');
  await act(async()=>[...host.querySelectorAll('button')].find(b=>b.textContent==='Browse conversation')!.click());await settle();
  await click('.back-to-list');
  expect(document.activeElement?.id).toBe(`activity-${id}`);
 }finally{await act(async()=>root.unmount());host.remove();}
});
