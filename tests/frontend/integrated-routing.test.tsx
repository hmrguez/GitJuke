import React,{act} from 'react';
import {createRoot} from 'react-dom/client';
import {Window} from 'happy-dom';
import {expect,test} from 'bun:test';

// Exercise the integrated shell, rather than replacing its route/receipt/log handlers.
async function routed(kind:'comment'|'check'|'summary'|'unavailable'){
 const dom=new Window({url:'http://localhost'});
 Object.assign(globalThis,{window:dom,document:dom.document,HTMLElement:dom.HTMLElement,FormData:dom.FormData,IS_REACT_ACT_ENVIRONMENT:true,isTauri:true,requestAnimationFrame:(cb:FrameRequestCallback)=>setTimeout(cb,0)});
 const head='a'.repeat(40),id='github.com:a/b:1:route-target';
 const pr={id:'PR_1',repo:'a/b',number:1,title:'Integrated route',url:'',body:'description',bodyLimited:false,head,base:'main',state:'open',draft:false,author:'reader',stack:{state:'none'}};
 const record={id,repo:'a/b',number:1,kind:kind==='check'?'check':'comment',occurred:1,actor:'other',url:'',text:'retained outcome',eligible:false,reason:'fixture',seen:false,head};
 const calls:{name:string;args:any}[]=[];
 let pending=true,routeKind=kind;
 const status={settings:{alerts:{onboarded:true,enabled:false,previews:false,categories:[true,true,true,true,true],permission:'notDetermined'},paused:true,pagedChanges:true,theme:{mode:'dark'}},identity:{id:42,login:'reader'},locked:false,health:'',revision:1,selected:null,storageStopped:false,ghPath:'/usr/local/bin/gh',configDir:'/Users/test/.config/gh',ghVersion:'raw-cli-version',storage:{warning:false,disposableBytes:12000000,managedBytes:20000000},watchCount:1,delayedWatches:0,discovery:[],budgets:[],ignoredOverrides:[],nextRemoteAt:0};
 (dom as any).__TAURI_INTERNALS__={invoke:async(name:string,args:any)=>{
  calls.push({name,args});
  if(name==='runtime_status')return status;
  if(name==='build_info')return {version:'9.8.7',buildId:'ci.345.2',revision:'a'.repeat(40),sourceState:'clean',target:'aarch64-apple-darwin',profile:'release',schemaVersion:16};
  if(name==='take_notification_route'){if(!pending)return null;pending=false;return {record:routeKind==='summary'||routeKind==='unavailable'?null:record,unavailable:routeKind==='unavailable'};}
  if(name==='saved_queries')return {items:[{id:7,name:'Local query',search:'is:open',version:1}],nextOffset:null,revision:1};
  if(name==='authored_page'||name==='requested_page')return {items:[pr],nextOffset:null,complete:true,revision:1};
  if(name==='activity_page')return {items:[record],nextOffset:null,revision:1,baseline:true,pending:0};
  if(name==='read_overview')return {data:pr,fetchedAt:1,complete:true,error:null,revision:1};
  if(name==='read_checks')return {items:[],commits:[],head,revision:1};
  if(name==='read_conversation')return {data:{head,source:args.job.source,generation:1,messages:[{id:'route-target',kind:'comment',author:'other',time:'now',url:null,body:'exact source',state:'complete',bytes:12,digest:'route-digest'}],threads:[],next:null,total:1,complete:true},revision:1,error:null};
  return null;
 }};
 const {App}=await import('../../src/app/App');
 const host=document.createElement('div');document.body.append(host);const root=createRoot(host);
 await act(async()=>{root.render(<App/>);await new Promise(r=>setTimeout(r,20));});
 await act(async()=>{await new Promise(r=>setTimeout(r,20));});
 return {host,calls,nextRoute:async(next:typeof kind)=>{routeKind=next;pending=true;await act(async()=>{await new Promise(r=>setTimeout(r,1100));});},reread:async()=>{await act(async()=>{dom.dispatchEvent(new dom.Event('focus'));await new Promise(r=>setTimeout(r,20));});},close:async()=>{await act(async()=>root.unmount());host.remove();}};
}

test('notification comment routes through the integrated exact reader and only its rendered receipt',async()=>{
 const r=await routed('comment');try{
  expect(r.host.textContent).toContain('Browse conversation');
  expect(r.host.textContent).toContain('exact source');
  expect(r.calls.filter(c=>c.name==='conversation_seen').map(c=>[c.args.id,c.args.digest])).toEqual([['route-target','route-digest']]);
  expect(r.calls.some(c=>['request_logs','activity_seen','enable_notifications','set_launch_at_login','set_pause'].includes(c.name))).toBe(false);
 }finally{await r.close();}
});
test('notification check opens Checks without automatically reading logs or enabling monitoring',async()=>{
 const r=await routed('check');try{
  expect(r.host.querySelector('#checks')).not.toBeNull();
  expect(r.calls.filter(c=>c.name==='select_pr').map(c=>c.args)).toEqual([{repo:'a/b',number:1}]);
  expect(r.calls.some(c=>['request_logs','conversation_seen','enable_notifications','set_launch_at_login','set_pause'].includes(c.name))).toBe(false);
 }finally{await r.close();}
});
test('summary and unavailable notification routes open Activity without a seen mutation',async()=>{
 for(const kind of ['summary','unavailable'] as const){const r=await routed(kind);try{
  expect(r.host.querySelector('nav button[title="Activity"]')?.classList.contains('active')).toBe(true);
  expect(r.host.textContent?.includes("This notification is no longer available.")).toBe(kind==='unavailable');
  await r.reread();
  expect(r.host.textContent?.includes("This notification is no longer available.")).toBe(kind==='unavailable');
  expect(r.calls.some(c=>['conversation_seen','activity_seen','request_logs','select_pr'].includes(c.name))).toBe(false);
 }finally{await r.close();}}
});

test('a later summary/unavailable route releases a previously opened check destination',async()=>{
 const r=await routed('check');try{
  expect(r.host.querySelector('dialog[open]')).not.toBeNull();
  const receipts=r.calls.filter(c=>c.name==='activity_rendered').length;
  await r.nextRoute('unavailable');
  expect(Boolean(r.host.querySelector('dialog[open]'))).toBe(false);
  expect(r.host.textContent).toContain("This notification is no longer available.");
  expect(r.calls.filter(c=>c.name==='activity_rendered')).toHaveLength(receipts);
 }finally{await r.close();}
});
test('Preferences close restores its opener when native activation did not move focus',async()=>{
 const r=await routed('summary');try{
  const opener=r.host.querySelector<HTMLButtonElement>('button[title="Preferences"]')!;
  r.host.querySelector<HTMLButtonElement>('nav button[title="Activity"]')!.focus();
  await act(async()=>opener.click());
  const dialog=r.host.querySelector<HTMLDialogElement>('dialog[open]')!;
  expect(Boolean(dialog)).toBe(true);
  await act(async()=>{dialog.close();await new Promise(resolve=>setTimeout(resolve,20));});
  expect(document.activeElement===opener).toBe(true);
 }finally{await r.close();}
});
test('query editor close restores the invoked New query control after native-style activation',async()=>{
 const r=await routed('summary');try{
  const opener=r.host.querySelector<HTMLButtonElement>('button[title="New saved search"]')!;
  r.host.querySelector<HTMLButtonElement>('nav button[title="Activity"]')!.focus();
  await act(async()=>opener.click());
  const dialog=r.host.querySelector<HTMLDialogElement>('dialog[open]')!;
  expect(Boolean(dialog)).toBe(true);
  await act(async()=>{dialog.close();await new Promise(resolve=>setTimeout(resolve,20));});
  expect(document.activeElement===opener).toBe(true);
 }finally{await r.close();}
});

test('preferences retain useful controls without showing runtime diagnostics',async()=>{
 const r=await routed('summary');try{
  const click=async(text:string)=>{await act(async()=>{[...r.host.querySelectorAll('button')].find(b=>b.textContent===text)!.click();});};
  await act(async()=>r.host.querySelector<HTMLButtonElement>('button[title="Preferences"]')!.click());
  await click('Updates');
  expect(r.host.textContent).toContain('Updates are paused.');expect(r.host.textContent).toContain('Enable notifications');
  for(const text of ['active watches','Discovery targets','payload','points','Registration:','launchd'])expect(r.host.textContent).not.toContain(text);
  await click('GitHub');expect(r.host.textContent).toContain('Connected as reader');expect(r.host.textContent).not.toContain('raw-cli-version');
  const form=r.host.querySelector('dialog form')!;
  await act(async()=>form.dispatchEvent(new window.Event('submit',{bubbles:true,cancelable:true})));
  expect(r.calls.find(c=>c.name==='configure_gh')?.args).toEqual({path:'/usr/local/bin/gh',config:'/Users/test/.config/gh'});
  await click('Storage');expect(r.host.textContent).toContain('Downloaded content: 12.0 MB');
  for(const text of ['SQLite','WAL','sidecar','ledger','replay','identity'])expect(r.host.textContent).not.toContain(text);
  await click('Clear downloaded content');expect(r.calls.some(c=>c.name==='clear_downloads')).toBe(true);
  await click('About');
  expect(r.host.textContent).toContain('GitJuke 9.8.7');expect(r.host.textContent).toContain('ci.345.2');
  expect(r.host.textContent).toContain('quit GitJuke with ⌘Q');
  expect(r.calls.filter(c=>c.name==='build_info')).toHaveLength(1);
  expect(r.calls.some(c=>['enable_notifications','set_launch_at_login'].includes(c.name))).toBe(false);
 }finally{await r.close();}
});
