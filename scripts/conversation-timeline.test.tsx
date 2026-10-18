import React,{act} from 'react';
import {createRoot,type Root} from 'react-dom/client';
import {Window} from 'happy-dom';
import {beforeEach,afterEach,expect,test} from 'bun:test';
import {Conversation} from '../src/Conversation';
import {ConversationTimeline,chronological,type TimelineMessage} from '../src/conversation-timeline';
import type {ConversationJob,ConversationPage,ConversationMessage,Overview,ReviewThread} from '../src/api';
const head='a'.repeat(40),pr={id:'PR',repo:'a/b',number:1,head,base:'b'.repeat(40),title:'Timeline',url:'',body:null,bodyLimited:false,state:'open',draft:false,author:'author',stack:{state:'none'}} as Overview;
const message=(id:string,time:string,body=id):ConversationMessage=>({id,time:`2026-10-18T${time}:00Z`,body,kind:id.startsWith('review')?'APPROVED':'comment',author:id,url:null,state:'complete',bytes:body.length,digest:id});
const thread:ReviewThread={id:'thread',path:'src/review.ts',line:8,side:'RIGHT',startLine:null,startSide:null,originalLine:null,originalStartLine:null,commit:head,originalCommit:null,outdated:false,resolved:true,url:null,time:'2026-10-18T12:00:00Z'};
const page=(job:ConversationJob):ConversationPage=>({head,source:job.source,generation:1,next:null,total:0,complete:true,messages:[],threads:[]});
let dom:Window,host:HTMLElement,root:Root,calls:{name:string;job:ConversationJob}[],read:(job:ConversationJob)=>Promise<any>;
const snapshot=(data:unknown)=>({data,error:null,revision:10,complete:true,fetchedAt:1});
const settle=()=>act(async()=>{await new Promise(resolve=>setTimeout(resolve,0));});
const render=async(node:React.ReactNode)=>{await act(async()=>root.render(node));await settle();await settle();};
beforeEach(()=>{
 dom=new Window();Object.assign(globalThis,{window:dom,document:dom.document,HTMLElement:dom.HTMLElement,isTauri:true,IS_REACT_ACT_ENVIRONMENT:true,IntersectionObserver:class{constructor(private callback:any){}observe(){this.callback([{isIntersecting:true}]);}disconnect(){}}});
 host=document.createElement('div');document.body.append(host);root=createRoot(host);calls=[];
 read=async job=>snapshot({...page(job),messages:job.source==='comments'?[message(job.after?'comment-later':'comment-first',job.after?'14:00':'10:00')]:job.source==='reviews'?[message('review-approved','13:00')]:job.source==='replies'?[message(job.after?'reply-later':'reply-first',job.after?'12:30':'12:00')]:[],threads:job.source==='threads'?[thread]:[],next:(job.source==='comments'||job.source==='replies')&&!job.after?'next':null});
 (dom as any).__TAURI_INTERNALS__={invoke:async(name:string,args:any)=>{calls.push({name,job:args?.job});if(name==='read_conversation')return read(args.job);}};
});
afterEach(async()=>{await act(async()=>root.unmount());host.remove();});

test('one chronological timeline automatically includes every comment and review page with inline thread replies',async()=>{
 await render(<Conversation pr={pr} currentHead={head} revision={1}/>);
 const entries=[...host.querySelectorAll('.conversation-timeline>.timeline-entry')];
 expect(entries.map(e=>e.textContent)).toEqual([expect.stringContaining('comment-first'),expect.stringContaining('src/review.ts'),expect.stringContaining('review-approved'),expect.stringContaining('comment-later')]);
 expect(host.querySelector('.timeline-thread')!.textContent).toContain('reply-first');
 expect(host.querySelector('.timeline-thread')!.textContent).toContain('reply-later');
 expect(host.textContent).toContain('Resolved');expect(host.textContent).toContain('Approved');
 for(const label of ['Review threads','Next page','Back to thread list','UTF-8','Source cursor'])expect(host.textContent).not.toContain(label);
 expect(host.querySelector('.discussion-sources')).toBeNull();expect(host.querySelector('#conversation h2')).toBeNull();expect(calls.some(c=>c.name==='conversation_seen')).toBe(false);
});

test('refresh replaces the timeline without retaining old comments or stealing keyboard focus',async()=>{
 await render(<Conversation pr={pr} currentHead={head} revision={1}/>);
 read=async job=>snapshot({...page(job),messages:job.source==='comments'?[message('fresh-comment','15:00')]:[]});
 const refresh=host.querySelector<HTMLButtonElement>('.reader-actions button')!;
 await act(async()=>{refresh.focus();refresh.click();});await settle();await settle();
 expect(host.textContent).toContain('fresh-comment');expect(host.textContent).not.toContain('comment-first');
 expect(document.activeElement).toBe(refresh);
 expect(calls.filter(c=>c.name==='request_conversation').length).toBeGreaterThan(0);
 expect(calls.some(c=>c.name==='conversation_seen')).toBe(false);
});

test('an authoritative removal clears the entire retained timeline and rejects a delayed page',async()=>{
 let resolve!:(value:any)=>void,blocked=false,removed=false;
 read=async job=>removed?snapshot(null):job.source==='reviews'?new Promise(r=>{blocked=true;resolve=r;}):snapshot({...page(job),messages:[message('private-comment','10:00')]});
 await render(<Conversation pr={pr} currentHead={head} revision={1}/>);expect(blocked).toBe(true);expect(host.textContent).toContain('private-comment');
 removed=true;await render(<Conversation pr={pr} currentHead={head} revision={2}/>);
 expect(host.textContent).not.toContain('private-comment');
 await act(async()=>resolve(snapshot({...page({...pr,source:'reviews'} as any),messages:[message('late-review','13:00')]})));await settle();
 expect(host.textContent).not.toContain('late-review');expect(host.querySelectorAll('.timeline-entry')).toHaveLength(0);
});

test('a failed discussion source does not hide other kinds of review activity',async()=>{
 read=async job=>job.source==='comments'?{...snapshot(null),error:'permission failed'}:snapshot({...page(job),messages:job.source==='reviews'?[message('review-visible','13:00')]:[]});
 await render(<Conversation pr={pr} currentHead={head} revision={1}/>);
 expect(host.textContent).toContain('review-visible');expect(host.textContent).toContain('Couldn’t load part of the discussion.');
});

test('message body cache evicts older bodies at the 100-message boundary while keeping metadata navigable',async()=>{
 const reader=new ConversationTimeline({repo:pr.repo,number:1,head},1);
 read=async job=>snapshot({...page(job),messages:[message(job.thread!,'10:00')]});
 const entries:TimelineMessage[]=Array.from({length:101},(_,i)=>({kind:'message',id:`message-${i}`,time:'',metadata:message(`message-${i}`,'10:00'),job:{repo:pr.repo,number:1,head,source:'replies',thread:`message-${i}`,after:null,seq:0}}));
 for(const entry of entries)await reader.show(entry);
 expect(reader.body(entries[0])).toBeUndefined();expect(reader.body(entries[100])?.body).toBe('message-100');
 await reader.show(entries[0]);expect(reader.body(entries[0])?.body).toBe('message-0');reader.close();
});

test('ordering uses timestamps rather than lexical dates and has deterministic ties',()=>{
 expect(chronological([{id:'b',time:'2026-10-18T14:00:00+02:00'},{id:'a',time:'2026-10-18T12:00:00Z'},{id:'c',time:'2026-10-18T11:00:00Z'}]).map(e=>e.id)).toEqual(['c','a','b']);
});

test('cold timeline loads use one native request at a time and do not cancel sibling readers',async()=>{
 const requested=new Set<string>();let active=0,peak=0;
 read=async job=>requested.has(job.source)?snapshot(page(job)):{...snapshot(null),error:'Queued — loading'};
 (dom as any).__TAURI_INTERNALS__.invoke=async(name:string,args:any)=>{
  calls.push({name,job:args?.job});
  if(name==='read_conversation')return read(args.job);
  if(name==='request_conversation'){active++;peak=Math.max(peak,active);await new Promise(r=>setTimeout(r,0));requested.add(args.job.source);active--;}
 };
 await render(<Conversation pr={pr} currentHead={head} revision={1}/>);await settle();await settle();
 expect(peak).toBe(1);expect([...requested]).toEqual(['comments','reviews','threads']);
 expect(calls.filter(c=>c.name==='cancel_conversation')).toHaveLength(0);
 expect(host.textContent).toContain('No discussion yet.');
});

test('body cache respects the UTF-8 byte ceiling even below the message-count ceiling',async()=>{
 const reader=new ConversationTimeline({repo:pr.repo,number:1,head},1);
 const body='é'.repeat(65536);
 read=async job=>snapshot({...page(job),messages:[message(job.thread!,'10:00',body)]});
 const entries:TimelineMessage[]=Array.from({length:17},(_,i)=>({kind:'message',id:`body-${i}`,time:'',metadata:message(`body-${i}`,'10:00'),job:{repo:pr.repo,number:1,head,source:'replies',thread:`body-${i}`,after:null,seq:0}}));
 for(const entry of entries)await reader.show(entry);
 expect(reader.body(entries[0])).toBeUndefined();expect(reader.body(entries[1])?.body).toBe(body);expect(reader.body(entries[16])?.body).toBe(body);reader.close();
});

test('a changed account generation clears previously indexed discussion instead of mixing accounts',async()=>{
 const reader=new ConversationTimeline({repo:pr.repo,number:1,head},1);
 read=async job=>snapshot({...page(job),generation:job.source==='comments'?1:2,messages:[message('private-'+job.source,'10:00')]});
 await reader.start();expect(reader.entries).toHaveLength(0);expect(reader.disposed).toBe(true);expect(reader.error).toBe(true);
});


test('comments, reviews and inline replies retain rich Markdown in ordinary discussions',async()=>{
 const body='# Review summary\n\n'+Array.from({length:20},(_,i)=>`- **Finding ${i+1}**: check the \`value\`.`).join('\n')+'\n\n## Verification\n\n> All checks passed.\n\n| Check | Result |\n| --- | --- |\n| Build | Passed |\n\n```ts\nconst value = 1;\n```';
 read=async job=>snapshot({...page(job),messages:['comments','reviews','replies'].includes(job.source)?[message(job.source,'10:00',body)]:[],threads:job.source==='threads'?[thread]:[]});
 await render(<Conversation pr={pr} currentHead={head} revision={1}/>);
 const bodies=host.querySelectorAll('.message-body');
 expect(bodies).toHaveLength(3);
 for(const body of bodies){
  expect(body.querySelector('h1')?.textContent).toBe('Review summary');
  expect(body.querySelector('h2')?.textContent).toBe('Verification');
  expect(body.querySelectorAll('li')).toHaveLength(20);
  expect(body.querySelectorAll('strong')).toHaveLength(20);
  expect(body.querySelector('blockquote')?.textContent).toContain('All checks passed.');
  expect(body.querySelector('table')).not.toBeNull();
  expect(body.querySelector('pre code')?.textContent).toBe('const value = 1;\n');
  expect(body.textContent).not.toContain('Formatting limit reached');
 }
});
