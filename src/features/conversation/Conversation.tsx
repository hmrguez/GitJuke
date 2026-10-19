import {Skeleton} from '../../components/Skeleton';
import React,{useEffect,useId,useLayoutEffect,useMemo,useRef,useState} from 'react';
import {command,type ConversationJob,type ConversationPage,type ConversationMessage,type Overview,type Snapshot} from '../../lib/ipc';
import {anchor,conversationTime,visitDestination,eligiblePage,segmentRanges,type PatchContext} from './conversation-model';
import './conversation.css';
import {ChevronDown,ChevronRight,Reply} from 'lucide-react';
import {Markdown} from '../../components/Markdown';
import {ConversationTimeline} from './ConversationTimeline';
type ConversationProps={pr:Overview;revision:number;currentHead:string;context?:PatchContext;inline?:boolean;destination?:string;onBack?:()=>void;onBrowse?:()=>void};
export function ConversationTab({active,...props}:ConversationProps&{active:boolean}){
 const [opened,setOpened]=useState(active);
 useEffect(()=>{if(active)setOpened(true);},[active]);
 return <div hidden={!active}>{(active||opened)&&<Conversation {...props}/>}</div>;
}
export function Conversation(props:ConversationProps){
 return props.destination?<DestinationConversation {...props}/>:<ConversationTimeline {...props}/>;
}
function DestinationConversation({pr,revision,currentHead,context,inline=false,destination,onBack,onBrowse}:ConversationProps){
 const [snapshot,setSnapshot]=useState<Snapshot<ConversationPage|null>|null>(null),[error,setError]=useState(''),[retry,setRetry]=useState(0);
 const serial=useRef(0),floor=useRef(revision),region=useRef<HTMLElement>(null);floor.current=revision;
 const job:ConversationJob={repo:pr.repo,number:pr.number,head:pr.head,source:'destination',thread:destination??null,after:null,seq:0};
 const key=JSON.stringify(job),[loadedFor,setLoadedFor]=useState('');
 const data=loadedFor===key?snapshot?.data:null;
 useEffect(()=>{
  const ticket=++serial.current;let active=true,busy=false;setSnapshot(null);setLoadedFor('');setError('Loading Conversation…');
  async function poll(){if(busy)return;busy=true;try{const p=await command<Snapshot<ConversationPage|null>>('read_conversation',{job});
   if(!active||ticket!==serial.current||p.revision<floor.current)return;
   if(p.data&&(!eligiblePage(p.data)||p.data.head!==pr.head||p.data.source!==job.source||(destination&&(p.data.messages.length!==1||p.data.messages[0].id!==destination))))throw new Error('Discussion page failed its source or retention bounds.');
   setSnapshot(p);setLoadedFor(key);setError(p.error??'');
  }catch(e){if(active){setSnapshot(null);setError(String(e));}}finally{busy=false;}}
  void(async()=>{try{const p=await command<Snapshot<ConversationPage|null>>('read_conversation',{job});if(!active)return;
   if(!p.data||p.error||retry)await command('request_conversation',{job});if(active)await poll();
  }catch(e){if(active)setError(String(e));}})();
  const timer=setInterval(()=>void poll(),1000);
  return()=>{active=false;clearInterval(timer);void command('cancel_conversation').catch(()=>{});};
 },[key,retry]);
 // Every new store revision rechecks the cache; denial/clear/account failures release retained bodies.
 useEffect(()=>{let active=true;void command<Snapshot<ConversationPage|null>>('read_conversation',{job}).then(p=>{if(active&&p.revision>=floor.current){if(p.data&&(!eligiblePage(p.data)||p.data.head!==pr.head||p.data.source!==job.source|| (destination&&(p.data.messages.length!==1||p.data.messages[0].id!==destination))))throw new Error('Conversation source or retention mismatch.');setSnapshot(p);setLoadedFor(key);setError(p.error??'');}}).catch(e=>{if(active){setSnapshot(null);setError(String(e));}});return()=>{active=false;};},[revision,key]);
 const destinationAnchor=data?.destinationAnchor;const destinationPosition=destinationAnchor?anchor(destinationAnchor,context):null;
 return <section id={inline?'inline-conversation':'conversation'} className="conversation" ref={region} tabIndex={-1} role={inline?'region':'tabpanel'} aria-label={inline?'Inline discussions':'Conversation'} aria-labelledby={inline?undefined:'conversation-tab'}>
  {destination&&<div className="reading-controls"><button onClick={onBack}>Back to Activity</button><button onClick={onBrowse}>Browse conversation</button>{destinationAnchor&&<span className="reader-meta">{destinationAnchor.path}{destinationAnchor.line?` · line ${destinationAnchor.line}`:''}</span>}{destinationPosition?.row!=null&&context&&<button onClick={()=>context.jump(destinationPosition.row!)}>Return to diff · line {destinationAnchor?.line}</button>}</div>}
  <div className="reader-actions"><button onClick={()=>setRetry(n=>n+1)}>Refresh conversation</button></div>
  {currentHead!==pr.head&&<p className="state-banner">New changes are available. Refresh the PR to see them.</p>}
  {error&&!/^(Loading|Queued)/.test(error)&&<p className="state-banner" role="status">{data?'Couldn’t refresh the conversation. Showing saved comments.':'Couldn’t load the conversation. Try refreshing.'}</p>}
  {!data&&(!error||/^(Loading|Queued)/.test(error))&&<Skeleton shape="conversation" label="Loading conversation"/>}
  <div className="message-page">{data?.messages.map(m=><Message key={`${key}:${m.id}:${m.digest}:${data.generation}`} message={m} job={job} generation={data.generation} explicitlyOpened={!!destination&&m.id===destination&&currentHead===pr.head} report={setError} nodeLimit={Math.min(4000,Math.floor((5000-data.messages.length*30)/Math.max(1,data.messages.length)))}/>)}</div>

 </section>;
}
const reviewLabels:Record<string,string>={APPROVED:'Approved',CHANGES_REQUESTED:'Requested changes',COMMENTED:'Reviewed',DISMISSED:'Review dismissed',PENDING:'Review pending',comment:'Commented',reply:'Replied',review:'Reviewed'};
// Timeline rows render nearby comments independently; the old 64-node page allowance
// made ordinary reviews fall back to literal source, including their headings.
export function Message({message:m,job,generation,report,explicitlyOpened=false,continuous=false,nodeLimit=continuous?4000:64,onReply,defaultExpanded=true,onExpandedChange}:{defaultExpanded?:boolean;onExpandedChange?:(expanded:boolean)=>void;onReply?:()=>void;continuous?:boolean;nodeLimit?:number;message:ConversationMessage;job:ConversationJob;generation:number;explicitlyOpened?:boolean;report:(e:string)=>void}){
 const [expanded,setExpanded]=useState(defaultExpanded),contentId=useId();
 const parts=useMemo(()=>continuous?[{start:0,end:(m.body??'').length}]:segmentRanges(m.body??''),[m.body,continuous]),[part,setPart]=useState(0),visited=useRef(new Set<number>()),element=useRef<HTMLElement>(null),body=useRef<HTMLDivElement>(null),opened=useRef(false),changingSegment=useRef(false);
 useLayoutEffect(()=>{if(changingSegment.current){if(body.current){body.current.scrollTop=0;body.current.focus();}changingSegment.current=false;}},[part]);
 useEffect(()=>{if(explicitlyOpened&&!opened.current){element.current?.focus();opened.current=true;}},[explicitlyOpened]);
 function changeSegment(delta:number){changingSegment.current=true;setPart(n=>n+delta);}
 useEffect(()=>{if(explicitlyOpened){if(visitDestination(m,m.id,part,parts.length,visited.current))void command('conversation_seen',{job,id:m.id,digest:m.digest,generation}).catch(e=>report(String(e)));}},[explicitlyOpened,part,m.id,m.body,m.digest,generation]);
 return <article ref={element} tabIndex={-1} className="conversation-message" aria-label={`${reviewLabels[m.kind]??'Comment'} by ${m.author??'unknown author'}`}><header><button className="message-toggle" type="button" aria-expanded={expanded} aria-controls={contentId} aria-label={`${expanded?'Collapse':'Expand'} comment by ${m.author??'unknown author'}`} onClick={()=>{onExpandedChange?.(!expanded);setExpanded(!expanded);}}>{expanded?<ChevronDown size={14} strokeWidth={1.65}/>:<ChevronRight size={14} strokeWidth={1.65}/>}</button><strong>{m.author??'Unknown author'}</strong>{['APPROVED','CHANGES_REQUESTED','COMMENTED','DISMISSED','PENDING'].includes(m.kind)&&<span>{reviewLabels[m.kind]}</span>}<time dateTime={m.time}>{conversationTime(m.time)}</time>{onReply&&<button type="button" className="reply-action" aria-label={`${job.source==='replies'?'Reply':'Quote reply'} to ${m.author??'comment'}`} onClick={onReply}><Reply size={13} strokeWidth={1.65}/>{job.source==='replies'?'Reply':'Quote reply'}</button>}</header>
  <div id={contentId} hidden={!expanded}>
  {m.state==='complete'&&m.body!==null?<><div ref={body} className="message-body" tabIndex={0} role="region" aria-label={`Message body${parts.length>1?` · segment ${part+1} of ${parts.length}`:''}`}><Markdown baseUrl={m.url??`https://github.com/${job.repo}/pull/${job.number}`} nodeLimit={nodeLimit}>{m.body.slice(parts[part].start,parts[part].end)}</Markdown></div>{parts.length>1&&<div className="pagination"><button disabled={part===0} onClick={()=>changeSegment(-1)}>Previous part</button><span>Part {part+1} of {parts.length}</span><button disabled={part===parts.length-1} onClick={()=>changeSegment(1)}>Next part</button></div>}</>:<p className="state-banner">{m.state==='oversized'?'This comment is too long to preview. View it on GitHub.':'This comment isn’t available.'}</p>}
  <div className="reading-controls">{m.url&&<button onClick={()=>void command('open_conversation',{job,id:m.id}).catch(e=>report(String(e)))}>View on GitHub</button>}</div>
 </div></article>;
}
