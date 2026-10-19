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

test('threads start expanded and collapse without losing a reply draft',async()=>{
 await render(<Conversation pr={pr} currentHead={head} revision={1}/>);
 const discussion=host.querySelector('.timeline-thread')!;
 const toggle=discussion.querySelector<HTMLButtonElement>('.thread-toggle')!;
 expect(toggle.getAttribute('aria-expanded')).toBe('true');
 const input=discussion.querySelector<HTMLTextAreaElement>('.comment-composer textarea')!;
 await act(async()=>{input.value='Keep this reply';input.dispatchEvent(new dom.Event('input',{bubbles:true}));toggle.click();});
 expect(toggle.getAttribute('aria-expanded')).toBe('false');
 expect(discussion.querySelector('[hidden]')).not.toBeNull();
 await act(async()=>toggle.click());
 expect(input.value).toBe('Keep this reply');
 expect(toggle.getAttribute('aria-expanded')).toBe('true');
});

test('new comments submit once and show the confirmed comment in the timeline',async()=>{
 let submissions=0,release!:(value:any)=>void;
 const invoke=(dom as any).__TAURI_INTERNALS__.invoke;
 (dom as any).__TAURI_INTERNALS__.invoke=async(name:string,args:any)=>{
  if(name==='post_conversation'){submissions++;expect(args.thread).toBeNull();expect(args.body).toBe('New discussion');return new Promise(r=>{release=r;});}
  return invoke(name,args);
 };
 await render(<Conversation pr={pr} currentHead={head} revision={1}/>);
 const form=host.querySelector<HTMLFormElement>('.conversation>.comment-composer')!;
 const input=form.querySelector<HTMLTextAreaElement>('textarea')!;
 const submit=form.querySelector<HTMLButtonElement>('[type=submit]')!;
 expect(submit.disabled).toBe(true);
 await act(async()=>{const setter=Object.getOwnPropertyDescriptor(dom.HTMLTextAreaElement.prototype,'value')!.set!;setter.call(input,'New discussion');input.dispatchEvent(new dom.Event('input',{bubbles:true}));});
 await act(async()=>{form.dispatchEvent(new dom.Event('submit',{bubbles:true,cancelable:true}));form.dispatchEvent(new dom.Event('submit',{bubbles:true,cancelable:true}));});
 expect(submissions).toBe(1);expect(submit.disabled).toBe(true);
 await act(async()=>release(message('posted-comment','16:00','New discussion')));await settle();
 expect(host.querySelector('.conversation-timeline')!.textContent).toContain('New discussion');
 expect(input.value).toBe('');expect(form.textContent).toContain('Comment posted.');
});

test('failed thread replies retain the draft and target the thread rather than the PR timeline',async()=>{
 const invoke=(dom as any).__TAURI_INTERNALS__.invoke;
 let target:unknown;
 (dom as any).__TAURI_INTERNALS__.invoke=async(name:string,args:any)=>{
  if(name==='post_conversation'){target=args.thread;throw new Error('Posting denied');}
  return invoke(name,args);
 };
 await render(<Conversation pr={pr} currentHead={head} revision={1}/>);
 const form=host.querySelector<HTMLFormElement>('.timeline-thread>.timeline-thread-heading+div>div>.comment-composer')!;
 const input=form.querySelector<HTMLTextAreaElement>('textarea')!;
 await act(async()=>{Object.getOwnPropertyDescriptor(dom.HTMLTextAreaElement.prototype,'value')!.set!.call(input,'Reply draft');input.dispatchEvent(new dom.Event('input',{bubbles:true}));});
 await act(async()=>form.dispatchEvent(new dom.Event('submit',{bubbles:true,cancelable:true})));await settle();
 expect(target).toBe('thread');expect(input.value).toBe('Reply draft');expect(form.querySelector('[role=alert]')?.textContent).toContain('Posting denied');
});

test('thread header Reply expands a collapsed discussion and focuses its existing draft',async()=>{
 await render(<Conversation pr={pr} currentHead={head} revision={1}/>);
 const discussion=host.querySelector('.timeline-thread')!;
 const toggle=discussion.querySelector<HTMLButtonElement>('.thread-toggle')!;
 const input=discussion.querySelector<HTMLTextAreaElement>('.comment-composer textarea')!;
 await act(async()=>{Object.getOwnPropertyDescriptor(dom.HTMLTextAreaElement.prototype,'value')!.set!.call(input,'Existing draft');input.dispatchEvent(new dom.Event('input',{bubbles:true}));toggle.click();});
 const reply=discussion.querySelector<HTMLButtonElement>('.timeline-thread-heading .reply-action');
 expect(reply).not.toBeNull();
 await act(async()=>reply!.click());
 expect(toggle.getAttribute('aria-expanded')).toBe('true');
 expect(document.activeElement).toBe(input);
 expect(input.value).toBe('Existing draft');
 expect(discussion.querySelectorAll('.comment-composer')).toHaveLength(1);
});

test('collapsed top-level comments offer a clearly labeled Quote reply with a focused composer',async()=>{
 await render(<Conversation pr={pr} currentHead={head} revision={1}/>);
 const entry=host.querySelector('.conversation-timeline>.timeline-entry')!;
 const toggle=entry.querySelector<HTMLButtonElement>('.message-toggle')!;
 await act(async()=>toggle.click());
 const reply=entry.querySelector<HTMLButtonElement>('.conversation-message header .reply-action');
 expect(reply).not.toBeNull();
 await act(async()=>reply!.click());
 const input=entry.querySelector<HTMLTextAreaElement>('.comment-composer textarea')!;
 expect(document.activeElement).toBe(input);
 expect(reply!.textContent).toBe('Quote reply');
 expect(entry.querySelector('.comment-composer')?.textContent).toContain('Posts a quoted comment to the PR conversation');
});

test('thread header Reply posts into that thread and displays the confirmed reply there',async()=>{
 const invoke=(dom as any).__TAURI_INTERNALS__.invoke;
 let submissions=0;
 (dom as any).__TAURI_INTERNALS__.invoke=async(name:string,args:any)=>{
  if(name==='post_conversation'){
   submissions++;
   expect(args.thread).toBe('thread');expect(args.body).toBe('Thanks, fixed in the latest commit.');
   return {...message('confirmed-reply','16:00',args.body),kind:'reply'};
  }
  return invoke(name,args);
 };
 await render(<Conversation pr={pr} currentHead={head} revision={1}/>);
 const discussion=host.querySelector('.timeline-thread')!;
 await act(async()=>discussion.querySelector<HTMLButtonElement>('.timeline-thread-heading .reply-action')!.click());
 const input=document.activeElement as HTMLTextAreaElement;
 const form=input.closest('form')!;
 await act(async()=>{Object.getOwnPropertyDescriptor(dom.HTMLTextAreaElement.prototype,'value')!.set!.call(input,'Thanks, fixed in the latest commit.');input.dispatchEvent(new dom.Event('input',{bubbles:true}));});
 await act(async()=>form.dispatchEvent(new dom.Event('submit',{bubbles:true,cancelable:true})));await settle();
 expect(submissions).toBe(1);expect(input.value).toBe('');
 expect(discussion.querySelector('.timeline-replies')!.textContent).toContain('Thanks, fixed in the latest commit.');
 expect(host.querySelectorAll('.conversation-timeline>.timeline-entry')).toHaveLength(4);
});

test('opening a conversation discovers review threads added after a cached empty thread list',async()=>{
 const refreshed=new Set<string>();
 const reviewThread={...thread,id:'PRRT_review',path:'docs/debug-pr.md',resolved:false};
 const rootComment={...message('PRRC_root','12:12',"I don't like this"),kind:'replies'};
 const existingReply={...message('PRRC_reply','12:35','qqq'),kind:'replies'};
 read=async job=>snapshot({...page(job),
  messages:job.source==='reviews'?[{...message('review-changes','12:12',''),kind:'CHANGES_REQUESTED'}]:job.source==='replies'?[rootComment,existingReply]:[],
  threads:job.source==='threads'&&refreshed.has('threads')?[reviewThread]:[]});
 const invoke=(dom as any).__TAURI_INTERNALS__.invoke;
 (dom as any).__TAURI_INTERNALS__.invoke=async(name:string,args:any)=>{
  if(name==='request_conversation')refreshed.add(args.job.source);
  if(name==='post_conversation'){
   expect(args.thread).toBe('PRRT_review');expect(args.body).toBe('Addressed your suggestion.');
   return {...message('PRRC_posted','13:00',args.body),kind:'reply'};
  }
  return invoke(name,args);
 };
 await render(<Conversation pr={pr} currentHead={head} revision={1}/>);
 const discussion=host.querySelector('.timeline-thread');
 expect(discussion).not.toBeNull();
 expect(discussion!.textContent).toContain("I don't like this");expect(discussion!.textContent).toContain('qqq');
 const input=discussion!.querySelector<HTMLTextAreaElement>('.comment-composer textarea')!;
 await act(async()=>{Object.getOwnPropertyDescriptor(dom.HTMLTextAreaElement.prototype,'value')!.set!.call(input,'Addressed your suggestion.');input.dispatchEvent(new dom.Event('input',{bubbles:true}));});
 await act(async()=>input.closest('form')!.dispatchEvent(new dom.Event('submit',{bubbles:true,cancelable:true})));await settle();
 expect(discussion!.textContent).toContain('Addressed your suggestion.');
 expect(host.querySelectorAll('.conversation-timeline>.timeline-entry')).toHaveLength(2);
});

test('fresh saved conversation pages are reused without unnecessary GitHub requests',async()=>{
 read=async job=>({...snapshot(page(job)),fetchedAt:Math.floor(Date.now()/1000)});
 await render(<Conversation pr={pr} currentHead={head} revision={1}/>);
 expect(calls.filter(c=>c.name==='request_conversation')).toHaveLength(0);
 expect(host.textContent).toContain('No discussion yet.');
});
