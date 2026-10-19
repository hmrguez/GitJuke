import type {ChangedFile,ConversationMessage,ConversationPage,ReviewThread} from '../../lib/ipc';
import type {DiffModel} from '../changes/diff-model';
export const BODY_LIMIT=131072,RETAINED_BYTES=2097152,RETAINED_MESSAGES=100,CONTENT_NODES=5000;
export function retentionWithin(messages:ConversationMessage[]):boolean{
 return messages.length<=RETAINED_MESSAGES&&messages.reduce((n,m)=>n+(m.body===null?0:new TextEncoder().encode(m.body).length),0)<=RETAINED_BYTES;
}
export function eligiblePage(p:ConversationPage):boolean{
 const messages:ConversationMessage[]=[];
 function check(page:ConversationPage,depth:number):boolean{
  if(depth>2||page.messages.length>50||page.threads.length>20)return false;
  messages.push(...page.messages);
  const seeds=page.seeds??[];
  if(page.source==='initial'){
   if(depth!==0||seeds.length!==3||seeds.some((seed,i)=>seed.thread!==null||seed.page.source!==['comments','reviews','threads'][i]))return false;
  }else if(seeds.length&&(page.source!=='threads'||seeds.length!==page.threads.length||seeds.some((seed,i)=>seed.thread!==page.threads[i].id||seed.page.source!=='replies')))return false;
  return seeds.every(seed=>seed.page.head===p.head&&seed.page.generation===p.generation&&check(seed.page,depth+1));
 }
 return check(p,0)&&retentionWithin(messages)&&messages.every(m=>m.body===null||new TextEncoder().encode(m.body).length<=BODY_LIMIT);
}
// Segment navigation bounds layout work; the Markdown renderer separately caps DOM nodes.
// Surrogate pairs are never split.
export function segmentRanges(body:string,size=32768):{start:number;end:number}[]{
 const result:{start:number;end:number}[]=[];for(let i=0;i<body.length;){let end=Math.min(body.length,i+size);if(end<body.length&&body.charCodeAt(end-1)>=0xd800&&body.charCodeAt(end-1)<=0xdbff)end--;result.push({start:i,end});i=end;}return result.length?result:[{start:0,end:0}];
}
export function segments(body:string,size=32768):string[]{return segmentRanges(body,size).map(r=>body.slice(r.start,r.end));}
export function canMarkSeen(m:Pick<ConversationMessage,'state'|'body'>,visited:number,count:number){return m.state==='complete'&&m.body!==null&&visited===count;}
export type PatchContext={file:ChangedFile;head:string;base:string;model:DiffModel;jump:(row:number)=>void};
export function anchor(t:ReviewThread,c?:PatchContext):{row:number|null;label:string}{
 if(t.outdated)return {row:null,label:`Outdated · original ${t.originalCommit?.slice(0,10)??'commit unavailable'}:${t.originalLine??'line unavailable'}`};
 if(!c)return {row:null,label:'Patch unavailable · select the matching file in Changes'};
 if(t.path!==c.file.path)return {row:null,label:'Different file · anchor unavailable in this patch'};
 if(t.side!=='LEFT'&&t.side!=='RIGHT')return {row:null,label:'Ambiguous side'};
 const commit=t.side==='LEFT'?c.base:c.head;
 if(t.commit!==commit)return {row:null,label:'Recorded commit does not match this patch side'};
 if(t.line===null||t.line<1)return {row:null,label:'File-level or unavailable line'};
 if(t.startLine!==null&&t.startSide!==t.side)return {row:null,label:'Ambiguous multi-side range'};
 const key=t.side==='LEFT'?'old':'next';
 const coordinates=new Map<number,{row:number;count:number}>();
 c.model.rows.forEach((r,row)=>{const line=r[key];if(line!==null){const previous=coordinates.get(line);coordinates.set(line,{row,count:(previous?.count??0)+1});}});
 const matches=coordinates.get(t.line);
 const start=t.startLine??t.line;
 if(start>t.line||t.line-start>20000)return {row:null,label:'Ambiguous range'};
 for(let line=start;line<=t.line;line++)if(coordinates.get(line)?.count!==1)return {row:null,label:'These lines aren’t available in the changes'};
 return matches?.count===1?{row:matches.row,label:`Attached · ${t.side.toLowerCase()} line ${t.line}`}:{row:null,label:'This line isn’t available in the changes'};
}
// Bounded metadata history only. Bodies are never kept in page/navigation history.
export function advance(history:(string|null)[],cursor:string|null): (string|null)[]{return [...history,cursor].slice(-128);}

// A commit contributes only to the explicitly opened matching source, never a neighbor.
export function visitDestination(m:ConversationMessage,destination:string|undefined,part:number,count:number,visited:Set<number>):boolean{
 if(!destination||m.id!==destination||!Number.isInteger(part)||part<0||part>=count)return false;
 visited.add(part);return canMarkSeen(m,visited.size,count);
}

const discussionDate=new Intl.DateTimeFormat(undefined,{dateStyle:'medium',timeStyle:'short'});
export function conversationTime(value:string):string{
 const date=new Date(value);return Number.isNaN(date.getTime())?value:discussionDate.format(date);
}
