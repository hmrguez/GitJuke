import {command,type ConversationJob,type ConversationMessage,type ConversationPage,type ReviewThread,type Snapshot} from './api';
import {eligiblePage,RETAINED_BYTES,RETAINED_MESSAGES} from './conversation-model';

export type TimelineMessage={kind:'message';id:string;time:string;metadata:Omit<ConversationMessage,'body'>;job:ConversationJob};
export type TimelineThread={kind:'thread';id:string;time:string;thread:ReviewThread;job:ConversationJob};
export type TimelineEntry=TimelineMessage|TimelineThread;
const jobKey=(job:ConversationJob)=>JSON.stringify([job.source,job.thread,job.after]);
const messageKey=(entry:TimelineMessage)=>`${jobKey(entry.job)}:${entry.id}`;
export function chronological<T extends {id:string;time:string}>(items:T[]):T[]{
 return [...items].sort((a,b)=>(Date.parse(a.time)||0)-(Date.parse(b.time)||0)||a.id.localeCompare(b.id));
}

// One foreground native request at a time. Timeline entries retain metadata only;
// bodies are a bounded, disposable cache shared by all visible discussion rows.
export class ConversationTimeline {
 entries:TimelineEntry[]=[];
 replies=new Map<string,TimelineMessage[]>();
 finished=false;
 error=false;
 disposed=false;
 revision:number;
 private generation:number|undefined;
 private version=0;
 private listeners=new Set<()=>void>();
 private tail:Promise<unknown>=Promise.resolve();
 private refreshed=new Set<string>();
 private pending=new Map<string,Promise<ConversationPage>>();
 private bodies=new Map<string,ConversationMessage>();
 private bytes=0;
 private timers=new Map<ReturnType<typeof setTimeout>,()=>void>();
 private replyLoads=new Map<string,Promise<void>>();
 constructor(readonly identity:Pick<ConversationJob,'repo'|'number'|'head'>,revision:number,private refresh=false){this.revision=revision;}
 subscribe=(fn:()=>void)=>{this.listeners.add(fn);return()=>{this.listeners.delete(fn);};};
 snapshot=()=>this.version;
 private emit(){if(!this.disposed){this.version++;this.listeners.forEach(fn=>fn());}}
 private wait(){return new Promise<void>(resolve=>{const timer=setTimeout(()=>{this.timers.delete(timer);resolve();},500);this.timers.set(timer,resolve);});}
 reportError(){this.error=true;this.emit();}
 private check(page:ConversationPage,job:ConversationJob){
  if(this.generation!==undefined&&this.generation!==page.generation){this.entries=[];this.replies.clear();this.error=true;this.emit();this.close();throw new Error('Conversation changed');}
  if(!eligiblePage(page)||page.head!==job.head||page.source!==job.source)throw new Error('Conversation changed');
  this.generation=page.generation;
 }
 private load(job:ConversationJob):Promise<ConversationPage>{
  const key=jobKey(job),pending=this.pending.get(key);if(pending)return pending;
  const work=this.tail.catch(()=>{}).then(async()=>{
   if(this.disposed)throw new Error('Closed');
   let value=await command<Snapshot<ConversationPage|null>>('read_conversation',{job});
   if(this.disposed)throw new Error('Closed');
   if(!value.data||value.error||(this.refresh&&!this.refreshed.has(key))){
    await command('request_conversation',{job});this.refreshed.add(key);
    do{
     if(this.disposed)throw new Error('Closed');
     value=await command<Snapshot<ConversationPage|null>>('read_conversation',{job});
     if(value.error&&!value.error.startsWith('Queued'))throw new Error(value.error);
     if(value.data&&!value.error&&value.revision>=this.revision)break;
     await this.wait();
    }while(!this.disposed);
   }
   if(this.disposed||!value.data||value.revision<this.revision)throw new Error('Unavailable');
   this.check(value.data,job);
   return value.data;
  });
  this.tail=work.then(()=>{}).catch(()=>{});this.pending.set(key,work);
  void work.finally(()=>this.pending.delete(key)).catch(()=>{});
  return work;
 }
 private messages(page:ConversationPage,job:ConversationJob):TimelineMessage[]{
  return page.messages.map(({body,...metadata})=>({kind:'message',id:metadata.id,time:metadata.time,metadata,job}));
 }
 private async pages(source:ConversationJob['source'],thread:string|null,receive:(page:ConversationPage,job:ConversationJob)=>void){
  let after:string|null=null;const visited=new Set<string>();
  do{
   const job:ConversationJob={...this.identity,source,thread,after,seq:0};
   const page=await this.load(job);if(this.disposed)return;
   receive(page,job);this.emit();
   after=page.next;
   if(after){if(visited.has(after))throw new Error('Repeated page');visited.add(after);}
  }while(after&&!this.disposed);
 }
 async start(){
  for(const source of ['comments','reviews','threads'] as const){
   if(this.disposed)return;
   try{
    await this.pages(source,null,(page,job)=>{
     const incoming:TimelineEntry[]=source==='threads'?page.threads.map(thread=>({kind:'thread',id:thread.id,time:thread.time,thread,job})):this.messages(page,job);
     const entries=new Map(this.entries.map(entry=>[`${entry.kind}:${entry.id}`,entry]));
     incoming.forEach(entry=>entries.set(`${entry.kind}:${entry.id}`,entry));
     this.entries=chronological([...entries.values()]);
    });
   }catch{if(!this.disposed){this.error=true;this.emit();}}
  }
  this.finished=true;this.emit();
 }
 body(entry:TimelineMessage){return this.bodies.get(messageKey(entry));}
 async show(entry:TimelineMessage){
  if(this.body(entry)||this.disposed)return;
  try{
   const page=await this.load(entry.job);if(this.disposed)return;
   const message=page.messages.find(message=>message.id===entry.id);if(!message)throw new Error('Removed');
   const key=messageKey(entry),bytes=new TextEncoder().encode(message.body??'').length;
   // An overlapping read of the same page may already have supplied this row.
   if(this.bodies.has(key))return;
   while(this.bodies.size&&(this.bodies.size>=RETAINED_MESSAGES||this.bytes+bytes>RETAINED_BYTES)){
    const oldest=this.bodies.keys().next().value!;this.bytes-=new TextEncoder().encode(this.bodies.get(oldest)?.body??'').length;this.bodies.delete(oldest);
   }
   this.bodies.set(key,message);this.bytes+=bytes;this.emit();
  }catch{if(!this.disposed){this.error=true;this.emit();}}
 }
 loadReplies(entry:TimelineThread){
  const existing=this.replyLoads.get(entry.id);if(existing)return existing;
  const work=this.pages('replies',entry.id,(page,job)=>{
   const entries=new Map((this.replies.get(entry.id)??[]).map(reply=>[reply.id,reply]));
   this.messages(page,job).forEach(reply=>entries.set(reply.id,reply));
   this.replies.set(entry.id,chronological([...entries.values()]));
  }).catch(()=>{if(!this.disposed){this.error=true;this.emit();}});
  this.replyLoads.set(entry.id,work);return work;
 }
 async validate(){
  if(this.disposed||!this.entries.length)return;
  try{
   const job:ConversationJob={...this.identity,source:'comments',thread:null,after:null,seq:0};
   const value=await command<Snapshot<ConversationPage|null>>('read_conversation',{job});
   if(this.disposed||value.revision<this.revision)return;
   if(!value.data)throw new Error('Removed');this.check(value.data,job);
  }catch{if(!this.disposed){this.entries=[];this.replies.clear();this.bodies.clear();this.bytes=0;this.error=true;this.emit();this.close();}}
 }
 close(){this.disposed=true;this.bodies.clear();this.bytes=0;this.timers.forEach((resolve,timer)=>{clearTimeout(timer);resolve();});this.timers.clear();}
}
