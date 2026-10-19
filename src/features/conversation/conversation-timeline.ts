import {command,type ConversationJob,type ConversationMessage,type ConversationPage,type ReviewThread,type Snapshot} from '../../lib/ipc';
import {eligiblePage,RETAINED_BYTES,RETAINED_MESSAGES} from './conversation-model';

export type TimelineMessage={kind:'message';id:string;time:string;metadata:Omit<ConversationMessage,'body'>;job:ConversationJob};
export type TimelineThread={kind:'thread';id:string;time:string;thread:ReviewThread;job:ConversationJob};
export type TimelineEntry=TimelineMessage|TimelineThread;
const jobKey=(job:ConversationJob)=>JSON.stringify([job.source,job.thread,job.after]);
const CONVERSATION_FRESH_MS=10_000;
const messageKey=(entry:Pick<TimelineMessage,'job'|'id'>)=>`${jobKey(entry.job)}:${entry.id}`;
export function chronological<T extends {id:string;time:string}>(items:T[]):T[]{
 return [...items].sort((a,b)=>(Date.parse(a.time)||0)-(Date.parse(b.time)||0)||a.id.localeCompare(b.id));
}

// One foreground native request at a time. Each page supplies metadata and bodies
// together; the bounded body cache only reloads comments after eviction.
export class ConversationTimeline {
 entries:TimelineEntry[]=[];
 replies=new Map<string,TimelineMessage[]>();
 collapsed=new Set<string>();
 finished=false;
 error=false;
 disposed=false;
 revision:number;
 private generation:number|undefined;
 private jobs=new Map<string,ConversationJob>();
 private validation:Promise<void>|undefined;
 private seeded=new Map<string,string|null>();
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
 private load(job:ConversationJob,force=false):Promise<ConversationPage>{
  const key=jobKey(job),pending=this.pending.get(key);if(pending)return pending;
  const work=this.tail.catch(()=>{}).then(async()=>{
   if(this.disposed)throw new Error('Closed');
   let value=await command<Snapshot<ConversationPage|null>>('read_conversation',{job});
   if(this.disposed)throw new Error('Closed');
   // Reviews and threads can change without a new commit. Revalidate saved
   // pages once per reader session instead of treating the head SHA as freshness.
   const stale=value.fetchedAt===null||Date.now()-value.fetchedAt*1000>=CONVERSATION_FRESH_MS;
   if(force||!value.data||value.error||((this.refresh||stale)&&!this.refreshed.has(key))){
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
   if(job.source!=='initial')this.jobs.set(key,job);
   for(const message of value.data.messages)this.retain(message,job);
   return value.data;
  });
  this.tail=work.then(()=>{}).catch(()=>{});this.pending.set(key,work);
  void work.finally(()=>this.pending.delete(key)).catch(()=>{});
  return work;
 }
 private retain(message:ConversationMessage,job:ConversationJob){
  const key=messageKey({job,id:message.id}),previous=this.bodies.get(key);
  if(previous){this.bytes-=new TextEncoder().encode(previous.body??'').length;this.bodies.delete(key);}
  const bytes=new TextEncoder().encode(message.body??'').length;
  while(this.bodies.size&&(this.bodies.size>=RETAINED_MESSAGES||this.bytes+bytes>RETAINED_BYTES)){
   const oldest=this.bodies.keys().next().value!;this.bytes-=new TextEncoder().encode(this.bodies.get(oldest)?.body??'').length;this.bodies.delete(oldest);
  }
  this.bodies.set(key,message);this.bytes+=bytes;
 }
 private messages(page:ConversationPage,job:ConversationJob):TimelineMessage[]{
  return page.messages.map(({body,...metadata})=>({kind:'message',id:metadata.id,time:metadata.time,metadata,job}));
 }
 private ingest(page:ConversationPage,job:ConversationJob){
  if(job.source!=='initial'){
   this.jobs.set(jobKey(job),job);
   for(const message of page.messages){
    const previous=this.bodies.get(messageKey({job,id:message.id}));
    if(previous?.digest!==message.digest||previous?.state!==message.state)this.retain(message,job);
   }
   if(job.source==='replies'){
    const entries=new Map((this.replies.get(job.thread!)??[]).map(entry=>[entry.id,entry]));
    this.messages(page,job).forEach(entry=>entries.set(entry.id,entry));
    this.replies.set(job.thread!,chronological([...entries.values()]));
   }else{
    const incoming:TimelineEntry[]=job.source==='threads'?page.threads.map(thread=>({kind:'thread',id:thread.id,time:thread.time,thread,job})):this.messages(page,job);
    const entries=new Map(this.entries.map(entry=>[`${entry.kind}:${entry.id}`,entry]));
    incoming.forEach(entry=>entries.set(`${entry.kind}:${entry.id}`,entry));
    this.entries=chronological([...entries.values()]);
   }
  }
  for(const seed of page.seeds??[]){
   const child:ConversationJob={...this.identity,source:seed.page.source as ConversationJob['source'],thread:seed.thread,after:null,seq:0};
   this.seeded.set(jobKey(child),seed.page.next);
   this.ingest(seed.page,child);
  }
 }
 private async pages(source:ConversationJob['source'],thread:string|null,force=false,after:string|null=null){
  const visited=new Set<string>();
  do{
   const job:ConversationJob={...this.identity,source,thread,after,seq:0};
   const page=await this.load(job,force);if(this.disposed)return;
   this.ingest(page,job);this.emit();
   after=page.next;
   if(after){if(visited.has(after))throw new Error('Repeated page');visited.add(after);}
  }while(after&&!this.disposed);
 }
 async start(){
  try{
   const job:ConversationJob={...this.identity,source:'initial',thread:null,after:null,seq:0};
   const initial=await this.load(job);if(this.disposed)return;
   // Index all initial sources and their root comments before notifying React.
   this.ingest(initial,job);this.emit();
   for(const seed of initial.seeds??[]){
    if(this.disposed)return;
    if(seed.page.next)try{await this.pages(seed.page.source as ConversationJob['source'],seed.thread,false,seed.page.next);}catch{if(!this.disposed)this.reportError();}
   }
  }catch{if(!this.disposed)this.reportError();}
  this.finished=true;this.emit();
 }
 body(entry:TimelineMessage){return this.bodies.get(messageKey(entry));}
 async show(entry:TimelineMessage){
  if(this.body(entry)||this.disposed)return;
  try{
   const page=await this.load(entry.job);if(this.disposed)return;
   if(!page.messages.some(message=>message.id===entry.id))throw new Error('Removed');
   this.emit();
  }catch{if(!this.disposed){this.error=true;this.emit();}}
 }
 loadReplies(entry:TimelineThread){
  const existing=this.replyLoads.get(entry.id);if(existing)return existing;
  const key=jobKey({...entry.job,source:'replies',thread:entry.id,after:null});
  const after=this.seeded.get(key);
  const work=(this.seeded.has(key)&&after===null?Promise.resolve():this.pages('replies',entry.id,false,after??null)).catch(()=>{if(!this.disposed)this.reportError();});
  this.replyLoads.set(entry.id,work);return work;
 }
 acceptPosted(message:ConversationMessage,thread:string|null=null){
  if(this.disposed)return;
  const job:ConversationJob={...this.identity,source:thread?'replies':'comments',thread,after:null,seq:0};
  const {body,...metadata}=message,entry:TimelineMessage={kind:'message',id:message.id,time:message.time,metadata,job};
  this.retain(message,job);
  if(thread)this.replies.set(thread,chronological([...(this.replies.get(thread)??[]),entry]));
  else this.entries=chronological([...this.entries,entry]);
  this.emit();
  void this.pages(thread?'replies':'comments',thread,true).catch(()=>this.reportError());
 }
 validate(){
  if(this.validation)return this.validation;
  const work=this.validatePages();this.validation=work;
  void work.finally(()=>{this.validation=undefined;}).catch(()=>{});
  return work;
 }
 private async validatePages(){
  if(this.disposed||!this.jobs.size)return;
  try{
   // Store/watch revisions update the existing index. They never restart the
   // reader or turn an unrelated revision into a GitHub refresh.
   const jobs=new Map(this.jobs);
   for(const [key,job] of jobs){
    const value=await command<Snapshot<ConversationPage|null>>('read_conversation',{job});
    if(this.disposed)return;
    if(value.revision<this.revision)continue;
    if(!value.data){if(this.jobs.has(key))throw new Error('Removed');continue;}
    this.check(value.data,job);this.jobs.set(key,job);
    if(value.data.next){
     const next={...job,after:value.data.next};
     if(!jobs.has(jobKey(next)))jobs.set(jobKey(next),next);
    }
    this.ingest(value.data,job);
   }
   this.emit();
  }catch{if(!this.disposed){this.entries=[];this.replies.clear();this.bodies.clear();this.bytes=0;this.error=true;this.emit();this.close();}}
 }

 close(){this.disposed=true;this.bodies.clear();this.bytes=0;this.timers.forEach((resolve,timer)=>{clearTimeout(timer);resolve();});this.timers.clear();}
}
