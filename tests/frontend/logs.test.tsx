import React,{act} from 'react';
import {createRoot,type Root} from 'react-dom/client';
import {Window} from 'happy-dom';
import {afterEach,beforeEach,expect,test} from 'bun:test';
import {CheckLog,LogBody} from '../../src/features/checks/CheckLog';
import type {CheckContext} from '../../src/lib/ipc';
const dom=new Window({url:'http://localhost'});
Object.assign(globalThis,{window:dom,document:dom.document,HTMLElement:dom.HTMLElement,IS_REACT_ACT_ENVIRONMENT:true,isTauri:true});
const job={repo:'a/b',number:1,head:'a'.repeat(40),check:'CR_0',offset:0};
const check={id:'CR_0',kind:'CheckRun',name:'CI',status:'COMPLETED',conclusion:'SUCCESS',url:'https://github.com/a/b/actions/runs/2/job/100',runId:2,attempt:1,commit:job.head} as CheckContext;
let root:Root,host:HTMLElement,calls:{name:string;args:any}[],reply:any;
const settle=()=>act(async()=>{await new Promise(r=>setTimeout(r,0));});
async function render(node:React.ReactNode){await act(async()=>root.render(node));await settle();}
async function click(label:string){await act(async()=>{Array.from(host.querySelectorAll('button')).find(b=>b.textContent===label)!.click();});await settle();}
beforeEach(()=>{host=document.createElement('div');document.body.append(host);root=createRoot(host);calls=[];reply={state:'complete',text:'private <script>log</script>\n'+Array.from({length:400},(_,i)=>`line ${i}`).join('\n')};(dom as any).__TAURI_INTERNALS__={invoke:async(name:string,args:any)=>{calls.push({name,args});if(name==='request_logs')return 12;if(name==='read_logs')return reply;return null;}};});
afterEach(async()=>{await act(async()=>root.unmount());host.remove();});
test('logs require explicit action, render safe bounded line pages and release on close',async()=>{
 await render(<CheckLog job={job} check={check} revision={1}/>);expect(calls).toHaveLength(0);
 await click('Show log');expect(host.querySelector('script')).toBeNull();expect(host.querySelector('pre')!.textContent).toContain('<script>');expect(host.querySelector('pre')!.textContent!.split('\n')).toHaveLength(200);
 await click('Next');expect(host.querySelector('pre')!.textContent).not.toContain('private');expect(document.activeElement===host.querySelector('pre')).toBe(true);
 await click('Hide log');expect(host.querySelector('pre')).toBeNull();expect(calls.filter(c=>c.name==='cancel_logs').map(c=>c.args.seq)).toEqual([12]);
});
test('external providers expose details without Actions affordance or automatic fetch',async()=>{
 await render(<CheckLog job={job} check={{...check,kind:'StatusContext',runId:null,attempt:null,url:'https://ci.example/job'}} revision={1}/>);expect(host.textContent).not.toContain('Show log');expect(calls).toHaveLength(0);
 await click('View details ↗');expect(calls.map(c=>c.name)).toEqual(['open_check_details']);
});
test('authoritative invalidation releases rendered text without requesting it again',async()=>{
 await render(<LogBody job={job} revision={1}/>);expect(host.querySelector('pre')).not.toBeNull();reply={state:'released',text:null};await render(<LogBody job={job} revision={2}/>);expect(host.querySelector('pre')).toBeNull();expect(calls.filter(c=>c.name==='request_logs')).toHaveLength(1);
});
test('late IPC responses after navigation cannot restore text and cancel only their own ticket',async()=>{
 let resolve!:(n:number)=>void;(dom as any).__TAURI_INTERNALS__.invoke=async(name:string,args:any)=>{calls.push({name,args});return name==='request_logs'?new Promise<number>(r=>resolve=r):null;};
 await render(<LogBody job={job} revision={1}/>);await render(null);await act(async()=>resolve(88));await settle();expect(host.textContent).toBe('');expect(calls.filter(c=>c.name==='read_logs')).toHaveLength(0);expect(calls.find(c=>c.name==='cancel_logs')!.args.seq).toBe(88);
});
test('a read crossing a newer revision cannot restore an obsolete body',async()=>{
 let resolve!:(v:any)=>void,reads=0;
 (dom as any).__TAURI_INTERNALS__.invoke=async(name:string,args:any)=>{calls.push({name,args});if(name==='request_logs')return 12;if(name==='read_logs'){if(++reads===1)return new Promise(r=>resolve=r);return {state:'released',text:null};}return null;};
 await render(<LogBody job={job} revision={1}/>);await render(<LogBody job={job} revision={2}/>);await act(async()=>resolve({state:'complete',text:'obsolete private text'}));await settle();
 expect(host.textContent).not.toContain('obsolete private text');expect(host.querySelector('pre')).toBeNull();expect(reads).toBe(2);
});
