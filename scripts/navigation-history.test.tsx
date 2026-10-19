import React,{act} from 'react';
import {createRoot} from 'react-dom/client';
import {Window} from 'happy-dom';
import {expect,test} from 'bun:test';

async function shell({mac=false,restored=false}={}){
  const dom=new Window({url:'http://localhost'});
  if(mac)Object.defineProperty(dom.navigator,'platform',{value:'MacIntel'});
  Object.assign(globalThis,{window:dom,document:dom.document,HTMLElement:dom.HTMLElement,FormData:dom.FormData,IS_REACT_ACT_ENVIRONMENT:true,isTauri:true,requestAnimationFrame:(cb:FrameRequestCallback)=>setTimeout(cb,0)});
  const prs=[1,2].map(number=>({id:`PR_${number}`,repo:'a/b',number,title:`Pull request ${number}`,url:'',body:`Body ${number}`,bodyLimited:false,head:'a'.repeat(40),base:'main',state:'open',draft:false,author:'reader',stack:{state:'none'}}));
  let queries=[{id:7,name:'Local query',search:'is:open',version:1}];
  const unavailable=new Set<number>();
  let queryRead:Promise<void>|null=null;
  const calls:{name:string;args:any}[]=[];
  const status={settings:{alerts:{onboarded:true},paused:true,pagedChanges:true,theme:{mode:'dark'}},identity:{id:42,login:'reader'},locked:false,revision:1,selected:restored?['a/b',1]:null,storageStopped:false,storage:{warning:false},discovery:[],budgets:[],ignoredOverrides:[]};
  (dom as any).__TAURI_INTERNALS__={invoke:async(name:string,args:any)=>{
    calls.push({name,args});
    if(name==='runtime_status')return status;
    if(name==='take_notification_route')return null;
    if(name==='saved_queries'){const pending=queryRead;queryRead=null;if(pending)await pending;return {items:queries,nextOffset:null,revision:1};}
    if(name==='authored_page'||name==='requested_page')return {items:prs,nextOffset:null,complete:true,revision:1};
    if(name==='activity_page')return {items:[],nextOffset:null,revision:1,baseline:true,pending:0};
    if(name==='read_overview')return {data:unavailable.has(args.number)?null:prs.find(pr=>pr.number===args.number)??null,revision:1};
    if(name==='read_checks')return {items:[],commits:[],head:'a'.repeat(40),revision:1};
    if(name==='query_page')return {items:prs,complete:true,total:2,nextPage:null,revision:1};
    return null;
  }};
  const {App}=await import('../src/main');
  const host=document.createElement('div');document.body.append(host);let root=createRoot(host);
  const settle=async(fn=()=>{})=>{await act(async()=>{fn();await new Promise(r=>setTimeout(r,10));});};
  await settle(()=>root.render(<App/>));
  const button=(label:string)=>host.querySelector<HTMLButtonElement>(`button[aria-label="${label}"]`)!;
  const click=async(label:string)=>settle(()=>button(label).click());
  const select=async(number:number)=>settle(()=>host.querySelector<HTMLButtonElement>(`#pr-PR_${number}`)!.click());
  const key=async(key:string,options:KeyboardEventInit={},target:Element=host)=>{
    const event=new dom.KeyboardEvent('keydown',{key,bubbles:true,cancelable:true,...options});
    await settle(()=>target.dispatchEvent(event));return event;
  };
  const mouse=async(button:number)=>{
    const events=['mousedown','mouseup','auxclick'].map(type=>new dom.MouseEvent(type,{button,bubbles:true,cancelable:true}));
    for(const event of events)await settle(()=>host.dispatchEvent(event));
    return events;
  };
  return {host,dom,calls,button,click,select,key,mouse,settle,unavailable,deferQueryRead:()=>{let release!:()=>void;queryRead=new Promise<void>(resolve=>{release=resolve;});return release;},deleteQueries:()=>{queries=[];},renameQuery:()=>{queries=[{...queries[0],name:'Updated query',version:2}];},remount:async()=>{await settle(()=>root.unmount());root=createRoot(host);await settle(()=>root.render(<App/>));},close:async()=>{await act(async()=>root.unmount());host.remove();}};
}

test('toolbar history crosses workspaces, restores reader tabs, branches and ignores duplicate selections',async()=>{
  const r=await shell();try{
    const toolbar=r.host.querySelector('.window-toolbar')!;
    expect([...toolbar.querySelectorAll('button')].slice(0,3).map(b=>b.getAttribute('aria-label'))).toEqual(['Go back','Go forward','Collapse sidebar']);
    expect(r.button('Go back').disabled).toBe(true);expect(r.button('Go forward').disabled).toBe(true);
    await r.select(1);
    await r.settle(()=>r.host.querySelector<HTMLButtonElement>('#checks-tab')!.click());
    await r.select(2);
    await r.click('Go back');
    expect(r.host.querySelector('.detail-toolbar')?.textContent).toContain('#1');
    expect(r.host.querySelectorAll('.pr-list .stack-node')).toHaveLength(2);
    expect(r.host.querySelector('#checks-tab')?.getAttribute('aria-selected')).toBe('true');
    expect(document.activeElement?.id).toBe('checks-tab');
    await r.click('Go back');
    expect(r.host.querySelector('.reader-empty')).not.toBeNull();expect(r.button('Go back').disabled).toBe(true);
    expect(document.activeElement?.id).toBe('pr-PR_1');
    await r.click('Go forward');
    expect(r.host.querySelector('.detail-toolbar')?.textContent).toContain('#1');
    await r.click('Review requests');
    expect(r.button('Go forward').disabled).toBe(true);
    await r.click('Review requests');
    await r.click('Go back');
    expect(r.button('My PRs').getAttribute('aria-current')).toBe('page');
    await r.click('Go forward');
    expect(r.button('Review requests').getAttribute('aria-current')).toBe('page');
  }finally{await r.close();}
});

test('Alt arrows and macOS Cmd brackets work from reader tabs and preserve editing and modal controls',async()=>{
  const r=await shell({mac:true});try{
    await r.select(1);
    const tab=r.host.querySelector('#overview-tab')!;
    expect((await r.key('ArrowLeft',{altKey:true},tab)).defaultPrevented).toBe(true);
    expect(r.host.querySelector('.reader-empty')).not.toBeNull();
    await r.key(']',{metaKey:true});
    expect(r.host.querySelector('.detail-toolbar')?.textContent).toContain('#1');
    const input=r.host.querySelector('input')!;
    expect((await r.key('[',{metaKey:true},input)).defaultPrevented).toBe(false);
    expect((await r.key('ArrowLeft',{altKey:true},input)).defaultPrevented).toBe(false);
    const editor=document.createElement('div');editor.contentEditable='true';r.host.append(editor);
    expect((await r.key('[',{metaKey:true},editor)).defaultPrevented).toBe(false);
    await r.click('Preferences');
    expect((await r.key('ArrowLeft',{altKey:true})).defaultPrevented).toBe(false);
    expect(r.host.querySelector('.detail-toolbar')?.textContent).toContain('#1');
  }finally{await r.close();}
});

test('mouse buttons 4 and 5 traverse once per gesture and suppress browser history at boundaries',async()=>{
  const r=await shell();try{
    await r.click('Review requests');await r.click('Activity');
    const back=await r.mouse(3);expect(back.every(e=>e.defaultPrevented)).toBe(true);
    expect(r.button('Review requests').getAttribute('aria-current')).toBe('page');
    await r.mouse(4);expect(r.button('Activity').getAttribute('aria-current')).toBe('page');
    expect((await r.mouse(4)).every(e=>e.defaultPrevented)).toBe(true);
    expect(r.button('Activity').getAttribute('aria-current')).toBe('page');
    const normal=new r.dom.MouseEvent('mouseup',{button:0,bubbles:true,cancelable:true});
    await r.settle(()=>r.host.dispatchEvent(normal));expect(normal.defaultPrevented).toBe(false);
  }finally{await r.close();}
});

test('deleted saved queries are skipped in both directions without reactivating them',async()=>{
  const r=await shell();try{
    await r.click('Local query');await r.click('Activity');
    r.deleteQueries();await r.click('Go back');
    expect(r.button('My PRs').getAttribute('aria-current')).toBe('page');
    expect(r.button('Go back').disabled).toBe(true);
    await r.click('Go forward');expect(r.button('Activity').getAttribute('aria-current')).toBe('page');
    expect(r.calls.filter(c=>c.name==='activate_query'&&c.args.id===7)).toHaveLength(1);
  }finally{await r.close();}
});

test('history starts with the restored selection and resets on remount',async()=>{
  const r=await shell({restored:true});try{
    expect(r.button('Go back').disabled).toBe(true);
    await r.click('Review requests');expect(r.button('Go back').disabled).toBe(false);
    await r.remount();expect(r.button('Go back').disabled).toBe(true);expect(r.button('Go forward').disabled).toBe(true);
  }finally{await r.close();}
});

test('query restoration uses the latest version and unavailable pull requests are skipped',async()=>{
  const r=await shell();try{
    await r.click('Local query');await r.click('Activity');r.renameQuery();
    await r.click('Go back');expect(r.host.querySelector('h1')?.textContent).toBe('Updated query');
    expect(r.calls.some(c=>c.name==='activate_query'&&c.args.id===7&&c.args.version===2)).toBe(true);
    await r.click('My PRs');await r.select(1);await r.select(2);r.unavailable.add(1);
    await r.click('Go back');expect(r.host.querySelector('.reader-empty')).not.toBeNull();
    expect(r.host.querySelectorAll('.pr-list .stack-node')).toHaveLength(2);
    await r.click('Go forward');expect(r.host.querySelector('.detail-toolbar')?.textContent).toContain('#2');
  }finally{await r.close();}
});

test('a delayed history destination cannot override newer workspace navigation',async()=>{
  const r=await shell();try{
    await r.click('Local query');await r.click('Activity');
    const first=r.deferQueryRead();await r.click('Go back');
    await r.key('ArrowRight',{altKey:true});await r.settle(first);
    expect(r.host.querySelector('[aria-label="Saved query results"]')).not.toBeNull();
    expect(r.button('Go forward').disabled).toBe(false);await r.click('Go forward');
    const release=r.deferQueryRead();
    await r.click('Go back');
    expect(r.button('Go back').disabled).toBe(true);
    await r.click('My PRs');await r.settle(release);
    expect(r.button('My PRs').getAttribute('aria-current')).toBe('page');
    expect(r.host.querySelector('[aria-label="Saved query results"]')).toBeNull();
    expect(r.button('Go forward').disabled).toBe(true);
  }finally{await r.close();}
});
