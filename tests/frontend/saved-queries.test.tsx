import React,{act} from 'react';
import {createRoot,type Root} from 'react-dom/client';
import {Window} from 'happy-dom';
import {test,expect,beforeEach,afterEach} from 'bun:test';
import {QueryEditor,QueryReader,QueryNavigation} from '../../src/features/queries/SavedQueries';
import type {QueryPage,SavedQuery} from '../../src/lib/ipc';
const dom=new Window({url:'http://localhost'});
Object.assign(globalThis,{window:dom,document:dom.document,HTMLElement:dom.HTMLElement,FormData:dom.FormData,IS_REACT_ACT_ENVIRONMENT:true,isTauri:true});
(dom.HTMLDialogElement.prototype as any).showModal=function(){this.open=true;};
const q:SavedQuery={id:1,version:1,name:'Review desk',search:'repo:a/b'};
const result:QueryPage={query:q,items:[{id:'PR1',repo:'a/b',repoId:null,number:1,title:'Saved result',url:'https://github.com/a/b/pull/1',state:'open',draft:false,author:'reader',updatedAt:'now'}],total:150,cachedCount:100,nextPage:2,fetchedAt:1,complete:false,error:null,revision:1};
let host:HTMLElement,root:Root,calls:{name:string;args:any}[],invoke:(name:string,args:any)=>Promise<any>;
async function settle(){await act(async()=>{await new Promise(r=>setTimeout(r,5));});}
async function render(node:React.ReactNode){await act(async()=>root.render(node));await settle();}
async function click(text:string){await act(async()=>Array.from(host.querySelectorAll('button')).find(b=>b.textContent===text)!.click());await settle();}
beforeEach(()=>{host=document.createElement('div');document.body.append(host);root=createRoot(host);calls=[];invoke=async(name)=>name==='query_page'?result:name==='saved_queries'?{items:[q],nextOffset:null,revision:1}:q;(dom as any).__TAURI_INTERNALS__={invoke:async(name:string,args:any)=>{calls.push({name,args});return invoke(name,args);}};});
afterEach(async()=>{await act(async()=>root.unmount());host.remove();});
test('native-style query form submits create, versioned edit and delete IPC',async()=>{
 let saved:SavedQuery|null=null,deleted=false;
 await render(<QueryEditor query={null} onSaved={v=>{saved=v;}} onDeleted={()=>{deleted=true;}} close={()=>{}}/>);
 const name=host.querySelector<HTMLInputElement>('[name=name]')!,search=host.querySelector<HTMLInputElement>('[name=search]')!;name.value='Review desk';search.value='repo:a/b';expect(document.activeElement===name).toBe(true);
 await act(async()=>host.querySelector('form')!.dispatchEvent(new dom.Event('submit',{bubbles:true,cancelable:true}) as any));await settle();expect(calls.find(c=>c.name==='save_query')?.args).toEqual({id:null,version:null,name:'Review desk',search:'repo:a/b'});expect(saved).toEqual(q);
 await render(<QueryEditor key="edit" query={q} onSaved={()=>{}} onDeleted={()=>{deleted=true;}} close={()=>{}}/>);await click('Save search');expect(calls.filter(c=>c.name==='save_query').at(-1)?.args.version).toBe(1);await click('Delete search');expect(deleted).toBe(true);expect(calls.find(c=>c.name==='delete_query')?.args).toEqual({id:1,version:1});
});
test('navigation selects persisted query and result opens standard PR reader; paging requests one page',async()=>{
 let selected:SavedQuery|null=null,pr='';await render(<QueryNavigation revision={1} current={null} onSelect={v=>{selected=v;}} onEdit={()=>{}} onCreate={()=>{}}/>);await click('Review desk');expect(selected).toEqual(q);
 await render(<QueryReader query={q} revision={1} locked={false} select={(repo,n)=>{pr=`${repo}#${n}`;}} edit={()=>{}}/>);expect(host.textContent).not.toContain('cached');await act(async()=>host.querySelector<HTMLButtonElement>('.stack-node')!.click());expect(pr).toBe('a/b#1');await click('Next results');expect(calls.filter(c=>c.name==='request_query').map(c=>c.args.page)).toEqual([1,2]);
});
test('obsolete async result cannot replace a newly edited query or locked content',async()=>{
 let resolve!:(v:QueryPage)=>void;invoke=async name=>name==='query_page'?new Promise<QueryPage>(r=>{resolve=r;}):null;
 await render(<QueryReader key="old" query={q} revision={1} locked={false} select={()=>{}} edit={()=>{}}/>);const old=resolve;
 invoke=async name=>name==='query_page'?{...result,query:{...q,version:2},items:[],total:0,cachedCount:0,complete:true,revision:2}:null;
 await render(<QueryReader key="new" query={{...q,version:2}} revision={2} locked={false} select={()=>{}} edit={()=>{}}/>);await act(async()=>old(result));await settle();expect(host.textContent).toContain('No matching pull requests');expect(host.textContent).not.toContain('Saved result');
 await render(<QueryReader key="new" query={{...q,version:2}} revision={3} locked select={()=>{}} edit={()=>{}}/>);expect(host.textContent).toContain('Check your GitHub connection');expect(host.querySelectorAll('.stack-node')).toHaveLength(0);
});

test('successful cached reads never erase a failed fetch or storage-protection message',async()=>{
 let revision=1;invoke=async(name)=>{if(name==='request_query')throw new Error('Local storage needs attention. Open Preferences to review it.');return name==='query_page'?{...result,revision,items:[],fetchedAt:null,total:null,cachedCount:0,complete:false}:null;};
 await render(<QueryReader query={q} revision={1} locked={false} health="Local storage needs attention. Open Preferences to review it." select={()=>{}} edit={()=>{}}/>);expect(host.textContent).toContain('Local storage needs attention. Open Preferences to review it.');expect(host.textContent).toContain('Local storage needs attention. Open Preferences to review it.');
 revision=2;await render(<QueryReader query={q} revision={2} locked={false} health="Local storage needs attention. Open Preferences to review it." select={()=>{}} edit={()=>{}}/>);expect(host.textContent).toContain('Local storage needs attention. Open Preferences to review it.');expect(host.querySelectorAll('.stack-node')).toHaveLength(0);
});

test('a loaded search page has no skeleton while other result pages remain uncached',async()=>{
 await render(<QueryReader query={q} revision={1} locked={false} select={()=>{}} edit={()=>{}}/>);
 expect(host.textContent).toContain('Saved result');expect(result.complete).toBe(false);
 expect(host.querySelector('.skeleton')).toBeNull();
});
