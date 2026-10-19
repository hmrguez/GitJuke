import React,{act} from 'react';
import {createRoot,type Root} from 'react-dom/client';
import {Window} from 'happy-dom';
import {afterEach,beforeEach,expect,test} from 'bun:test';
import {Startup,type StartupReport} from '../../src/components/Startup';
const dom=new Window({url:'http://localhost'});
Object.assign(globalThis,{window:dom,document:dom.document,HTMLElement:dom.HTMLElement,IS_REACT_ACT_ENVIRONMENT:true,isTauri:true});
let root:Root,host:HTMLElement,calls:string[],report:StartupReport;
const settle=()=>act(async()=>{await new Promise(r=>setTimeout(r,0));});
beforeEach(()=>{host=document.createElement('div');document.body.append(host);root=createRoot(host);calls=[];report={ready:false,databasePath:'/disposable/db',message:'Storage protection',databaseState:'corrupt'};(dom as any).__TAURI_INTERNALS__={invoke:async(name:string)=>{calls.push(name);return {...report};}};});
afterEach(async()=>{await act(async()=>root.unmount());host.remove();});
test('failed startup exposes actionable location without mounting private readers; retry only mounts after success',async()=>{
 await act(async()=>root.render(<Startup><p>private reader</p></Startup>));await settle();
 expect(host.textContent).toContain('/disposable/db');expect(host.textContent).toContain('damaged');expect(host.textContent).not.toContain('private reader');expect(calls).toEqual(['startup_status','build_info']);
 await act(async()=>host.querySelector('button')!.click());await settle();expect(host.textContent).not.toContain('private reader');
 report.ready=true;await act(async()=>host.querySelector('button')!.click());await settle();expect(host.textContent).toBe('private reader');expect(calls).toEqual(['startup_status','build_info','retry_startup','retry_startup']);
});
test('all startup input states have distinct honest guidance',async()=>{
 for(const [databaseState,guidance] of [['missing','Your saved data couldn’t be found'],['legacy','needs an update before GitJuke can open it'],['current','couldn’t be opened. Try again after checking storage'],['newer','Update GitJuke to open data saved by a newer version'],['corrupt','appears damaged'],['unreadable','can’t access the folder']] as const){
  report.databaseState=databaseState;
  await act(async()=>root.render(<Startup key={databaseState}><p>private reader</p></Startup>));await settle();
  expect(host.textContent).toContain(guidance);expect(host.textContent).toContain('Try again');expect(host.textContent).not.toContain('private reader');expect(host.textContent).toContain('Your saved data won’t be reset');
 }
});

test('late startup status cannot replace a successful newer retry',async()=>{
 let complete!:(r:StartupReport)=>void;
 (dom as any).__TAURI_INTERNALS__={invoke:async(name:string)=>name==='startup_status'?new Promise<StartupReport>(r=>{complete=r;}):{...report,ready:true}};
 await act(async()=>root.render(<Startup><p>private reader</p></Startup>));await settle();
 await act(async()=>host.querySelector('button')!.click());await settle();expect(host.textContent).toBe('private reader');
 await act(async()=>complete(report));await settle();expect(host.textContent).toBe('private reader');
});
