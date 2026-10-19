import React,{act} from 'react';
import {createRoot} from 'react-dom/client';
import {Window} from 'happy-dom';
import {expect,test} from 'bun:test';

// Exercise the real Tauri event subscription, IPC state sync, and integrated reader.
test('native menus dispatch current actions, synchronize modal and selection state, and clean up',async()=>{
 const dom=new Window({url:'http://localhost'});
 Object.defineProperty(dom.navigator,'platform',{value:'MacIntel'});
 Object.assign(globalThis,{window:dom,document:dom.document,HTMLElement:dom.HTMLElement,FormData:dom.FormData,IS_REACT_ACT_ENVIRONMENT:true,isTauri:true,requestAnimationFrame:(cb:FrameRequestCallback)=>setTimeout(cb,0)});
 const callbacks=new Map<number,(event:any)=>void>(),listeners=new Map<string,number>();
 let next=0;
 const calls:{name:string;args:any}[]=[];
 const pr={id:'pr-1',repo:'a/b',number:1,title:'Menu test',url:'https://github.com/a/b/pull/1',head:'a'.repeat(40),body:'body',stack:{state:'none'},state:'open'};
 const status={settings:{alerts:{onboarded:true},theme:{mode:'dark'},paused:false},storage:{warning:false},locked:false,storageStopped:false,revision:1,selected:null,identity:null};
 (dom as any).__TAURI_EVENT_PLUGIN_INTERNALS__={unregisterListener:()=>{}};
 (dom as any).__TAURI_INTERNALS__={transformCallback:(fn:any)=>{callbacks.set(++next,fn);return next;},invoke:async(name:string,args:any)=>{
  calls.push({name,args});
  if(name==='plugin:event|listen'){listeners.set(args.event,args.handler);return args.handler;}
  if(name==='plugin:event|unlisten'){callbacks.delete(args.eventId);return;}
  if(name==='runtime_status')return status;
  if(name==='take_notification_route')return null;
  if(name==='authored_page'||name==='requested_page')return {items:[pr],complete:true,revision:1};
  if(name==='read_overview')return {data:pr,revision:1};
  if(name==='read_checks')return {items:[],commits:[],revision:1,head:pr.head};
  if(name==='saved_queries'||name==='activity_page')return {items:[],revision:1};
  if(name==='build_info')return {version:'1',buildId:'test',revision:'abc',sourceState:'clean'};
 }};
 const {App}=await import('../src/main');
 const host=document.createElement('div');document.body.append(host);const root=createRoot(host);
 const flush=async()=>{await new Promise(r=>setTimeout(r,15));};
 const dispatch=async(payload:string)=>{await act(async()=>{callbacks.get(listeners.get('reader-menu')!)!({payload});await flush();});};
 const state=()=>calls.filter(c=>c.name==='sync_reader_menu').at(-1)?.args.state;
 try{
  await act(async()=>{root.render(<App/>);await flush();});
  await act(flush);
  expect(state()).toMatchObject({selected:false,modal:false,filter:true,textSize:100,canMonitor:true});
  await dispatch('reader-open-pr');expect(calls.some(c=>c.name==='open_pr')).toBe(false);
  await act(async()=>{host.querySelector<HTMLButtonElement>('.stack-node')!.click();await flush();});
  await act(flush);expect(state().selected).toBe(true);
  await dispatch('reader-open-pr');expect(calls.find(c=>c.name==='open_pr')?.args).toEqual({repo:'a/b',number:1});
  await dispatch('reader-sidebar');expect(host.querySelector('.sidebar.is-collapsed')).not.toBeNull();expect(state().collapsed).toBe(true);
  await dispatch('reader-review');expect(host.querySelector('nav button[title="Review requests"]')?.getAttribute('aria-current')).toBe('page');
  dom.matchMedia=(()=>({matches:true})) as typeof dom.matchMedia;
  await dispatch('reader-filter');expect(document.activeElement?.getAttribute('aria-label')).toBe('Filter pull requests');expect(state().selected).toBe(false);
  await dispatch('reader-refresh');expect(calls.some(c=>c.name==='refresh')).toBe(true);
  await dispatch('reader-pause');expect(calls.find(c=>c.name==='set_pause')?.args).toEqual({paused:true});
  await dispatch('reader-about');await act(flush);
  expect(host.querySelector('section[aria-label="About"]')).not.toBeNull();expect(state().modal).toBe(true);
  await dispatch('reader-add');expect(host.querySelectorAll('dialog')).toHaveLength(1);
  await dispatch('reader-larger');expect(state().textSize).toBe(125);
  await dispatch('reader-actual');expect(state().textSize).toBe(100);
  await act(async()=>{host.querySelector<HTMLButtonElement>('button[aria-label="Close preferences"]')!.click();await flush();});
  await act(flush);expect(state().modal).toBe(false);
  await dispatch('reader-activity');expect(state().filter).toBe(false);
  await dispatch('reader-query');expect(host.textContent).toContain('New saved search');
 }finally{await act(async()=>root.unmount());host.remove();}
 expect(callbacks.has(listeners.get('reader-menu')!)).toBe(false);
});
