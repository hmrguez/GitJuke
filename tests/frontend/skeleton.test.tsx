import React,{act} from 'react';
import {createRoot,type Root} from 'react-dom/client';
import {Window} from 'happy-dom';
import {beforeEach,afterEach,expect,test} from 'bun:test';
import {BuildInformation} from '../../src/features/preferences/BuildInformation';
import {QueryReader} from '../../src/features/queries/SavedQueries';
import {LogBody} from '../../src/features/checks/CheckLog';
import {ActivityReader} from '../../src/features/activity/Activity';
import type {BuildInfo,SavedQuery} from '../../src/lib/ipc';

let dom:Window,host:HTMLElement,root:Root,invoke:(name:string)=>Promise<unknown>;
const pending=()=>new Promise<never>(()=>{});
const settle=()=>act(async()=>{await new Promise(resolve=>setTimeout(resolve,0));});
beforeEach(()=>{
 dom=new Window();Object.assign(globalThis,{window:dom,document:dom.document,isTauri:true,IS_REACT_ACT_ENVIRONMENT:true});
 host=document.createElement('div');document.body.append(host);root=createRoot(host);invoke=pending;
 (dom as any).__TAURI_INTERNALS__={invoke:(name:string)=>invoke(name)};
});
afterEach(async()=>{await act(async()=>root.unmount());host.remove();});
test('About reserves build metadata then replaces it with the resolved information',async()=>{
 let resolve!:(value:BuildInfo)=>void;
 invoke=()=>new Promise<BuildInfo>(done=>{resolve=done;});
 await act(async()=>root.render(<BuildInformation/>));
 expect(host.querySelector('.skeleton-build')).not.toBeNull();
 expect(host.querySelector('[role="status"]')?.getAttribute('aria-label')).toBe('Loading build information');
 expect(host.querySelector('.skeleton-content')?.getAttribute('aria-hidden')).toBe('true');
 expect(host.querySelector('.skeleton button,.skeleton [tabindex]')).toBeNull();
 await act(async()=>resolve({version:'1.2.3',buildId:'local',revision:'abc',sourceState:'clean',target:'macOS',profile:'debug',schemaVersion:1}));
 expect(host.querySelector('.skeleton')).toBeNull();expect(host.textContent).toContain('1.2.3');
});
test('saved searches show list skeletons before the first result and preserve connection guidance',async()=>{
 const query:SavedQuery={id:1,version:1,name:'Review',search:'is:pr'};
 const props={query,revision:0,select:()=>{},edit:()=>{}};
 await act(async()=>root.render(<QueryReader {...props} locked={false}/>));
 expect(host.querySelector('.skeleton-pull-requests')).not.toBeNull();
 await act(async()=>root.render(<QueryReader {...props} locked/>));
 expect(host.querySelector('.skeleton')).toBeNull();expect(host.textContent).toContain('Check your GitHub connection');
});
test('Activity uses row skeletons while waiting and removes them on a settled empty page',async()=>{
 let resolve!:(value:unknown)=>void;invoke=()=>new Promise(done=>{resolve=done;});
 await act(async()=>root.render(<ActivityReader locked={false} revision={0} report={()=>{}} open={()=>{}}/>));
 expect(host.querySelector('.skeleton-activity')).not.toBeNull();
 await act(async()=>resolve({revision:0,items:[],baseline:true,recovery:false,nextOffset:null,error:null}));
 expect(host.querySelector('.skeleton')).toBeNull();expect(host.textContent).toContain('You’re all caught up.');
});
test('queued logs show code lines and terminal failures replace the skeleton with guidance',async()=>{
 const job={repo:'a/b',number:1,head:'a',check:'one',offset:0};
 await act(async()=>root.render(<LogBody job={job} revision={0}/>));
 expect(host.querySelector('.skeleton-log')).not.toBeNull();
 invoke=async()=>{throw new Error('denied');};
 await act(async()=>root.render(<LogBody job={{...job,check:'two'}} revision={0}/>));await settle();
 expect(host.querySelector('.skeleton')).toBeNull();expect(host.textContent).toContain('Couldn’t load this log');
});
