import React,{act} from 'react';
import {createRoot} from 'react-dom/client';
import {Window} from 'happy-dom';
import {expect,test} from 'bun:test';
import {mockIPC,clearMocks} from '@tauri-apps/api/mocks';
import {readAllChecks} from '../../src/features/checks/check-pages';
import {CheckReader} from '../../src/features/checks/Checks';
import type {CheckContext,CheckPage} from '../../src/lib/ipc';

const head='a'.repeat(40);
const context=(id:string):CheckContext=>({id,name:id,kind:'CheckRun',status:'COMPLETED',conclusion:'SUCCESS',commit:head,runId:1,attempt:1,url:`https://github.com/a/b/actions/runs/1/job/${id}`,suiteId:null,suiteStatus:null,suiteConclusion:null});
const page=(items:CheckContext[]):CheckPage=>({pr:null,error:null,review:null,head,revision:1,nextOffset:null,commits:[{resource:head,complete:true,error:null,fetchedAt:1}],items});
const flush=()=>act(async()=>{await new Promise(resolve=>setTimeout(resolve,0));});

test('initial checks loading drains every local page and rejects a changing snapshot',async()=>{
 Object.assign(globalThis,{isTauri:true});
 const offsets:number[]=[];
 mockIPC((name,args)=>{
  expect(name).toBe('read_checks');const offset=args!.offset as number;offsets.push(offset);
  return {...page([context(String(offset))]),nextOffset:offset<200?offset+100:null};
 });
 try{
  const result=await readAllChecks({repo:'a/b',number:1});
  expect(offsets).toEqual([0,100,200]);expect(result.items.map(item=>item.id)).toEqual(['0','100','200']);expect(result.nextOffset).toBeNull();
  mockIPC((_,args)=>({...page([]),revision:args!.offset===0?1:2,nextOffset:args!.offset===0?100:null}));
  await expect(readAllChecks({repo:'a/b',number:1})).rejects.toThrow('Checks changed');
 }finally{clearMocks();}
});

test('hidden Checks preloads all Actions logs serially and shows them without another request',async()=>{
 const dom=new Window();
 Object.assign(globalThis,{window:dom,document:dom.document,isTauri:true,IS_REACT_ACT_ENVIRONMENT:true});
 const requests:string[]=[],cancelled:number[]=[];
 let current:string|undefined,seq=0;
 mockIPC((name,args)=>{
  if(name==='request_logs'){expect(current).toBeUndefined();current=(args!.job as any).check;requests.push(current!);return ++seq;}
  if(name==='read_logs')return {state:'complete',text:`Log ${current}`};
  if(name==='cancel_logs'){cancelled.push(args!.seq as number);current=undefined;}
 });
 const host=document.createElement('div');document.body.append(host);const root=createRoot(host);
 const render=(checks:CheckPage)=><div hidden><CheckReader repo="a/b" number={1} checks={checks} revision={checks.revision} offset={0} setOffset={()=>{}}/></div>;
 try{
  await act(async()=>root.render(render(page([context('1'),context('2')]))));await flush();
  expect(requests).toEqual(['1','2']);expect(cancelled).toEqual([1,2]);
  await act(async()=>host.querySelector<HTMLButtonElement>('button')!.click());
  expect(host.textContent).toContain('Log 1');expect(requests).toEqual(['1','2']);
  const updated=page([context('1'),context('2'),context('3')]);updated.revision=2;updated.commits[0].fetchedAt=2;
  await act(async()=>root.render(render(updated)));await flush();
  expect(requests).toEqual(['1','2','3']);
  await act(async()=>root.render(render({...updated,revision:3,commits:[{...updated.commits[0],fetchedAt:3}]})));await flush();
  expect(requests).toEqual(['1','2','3']);
 }finally{await act(async()=>root.unmount());host.remove();clearMocks();}
});

test('leaving a PR cancels a late log ticket without retaining its body',async()=>{
 const dom=new Window();
 Object.assign(globalThis,{window:dom,document:dom.document,isTauri:true,IS_REACT_ACT_ENVIRONMENT:true});
 let finish!:(seq:number)=>void,reads=0;const cancellations:number[]=[];
 mockIPC((name,args)=>{
  if(name==='request_logs')return new Promise<number>(resolve=>{finish=resolve;});
  if(name==='read_logs'){reads++;return {state:'complete',text:'late private log'};}
  if(name==='cancel_logs')cancellations.push(args!.seq as number);
 });
 const host=document.createElement('div');document.body.append(host);const root=createRoot(host);
 try{
  await act(async()=>root.render(<CheckReader repo="a/b" number={1} checks={page([context('1')])} revision={1} offset={0} setOffset={()=>{}}/>));
  await act(async()=>root.unmount());
  await act(async()=>finish(7));await flush();
  expect(cancellations).toEqual([7]);expect(reads).toBe(0);expect(host.textContent).toBe('');
 }finally{host.remove();clearMocks();}
});
