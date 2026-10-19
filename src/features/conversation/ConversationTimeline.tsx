import {Skeleton} from '../../components/Skeleton';
import React,{useEffect,useId,useLayoutEffect,useRef,useState,useSyncExternalStore} from 'react';
import {command,type Overview} from '../../lib/ipc';
import {anchor,conversationTime,type PatchContext} from './conversation-model';
import {ConversationTimeline as Timeline,type TimelineEntry,type TimelineMessage,type TimelineThread} from './conversation-timeline';
import {ChevronDown,ChevronRight,Reply} from 'lucide-react';
import {CommentComposer} from './CommentComposer';
import {Message} from './Conversation';

type Props={pr:Overview;revision:number;currentHead:string;context?:PatchContext;inline?:boolean};
export function ConversationTimeline(props:Props){
 const [refresh,setRefresh]=useState(0);
 return <section id={props.inline?'inline-conversation':'conversation'} className="conversation" tabIndex={-1} role={props.inline?'region':'tabpanel'} aria-label={props.inline?'Inline discussions':'Conversation'} aria-labelledby={props.inline?undefined:'conversation-tab'}>
  <div className="reader-actions"><button onClick={()=>setRefresh(n=>n+1)}>Refresh conversation</button></div>
  {props.currentHead!==props.pr.head&&<p className="state-banner">New changes are available. Refresh the PR to see them.</p>}
  <TimelineRevision key={`${props.pr.repo}:${props.pr.number}:${props.pr.head}:${refresh}`} {...props} refresh={refresh>0}/>
 </section>;
}
function TimelineRevision({pr,revision,context,refresh}:Props&{refresh:boolean}){
 const [timeline]=useState(()=>new Timeline({repo:pr.repo,number:pr.number,head:pr.head},revision,refresh));
 timeline.revision=revision;
 useSyncExternalStore(timeline.subscribe,timeline.snapshot);
 useEffect(()=>{void timeline.start();return()=>{timeline.close();void command('cancel_conversation').catch(()=>{});};},[timeline]);
 useEffect(()=>{void timeline.validate();},[revision,timeline]);
 return <>
  {timeline.error&&<p className="state-banner" role="status">Couldn’t load part of the discussion. Refresh to try again.</p>}
  <div className="conversation-timeline" aria-label="Discussion timeline">{timeline.entries.map(entry=><TimelineRow key={`${entry.kind}:${entry.id}`} {...{entry,timeline,context}}/>)}</div>
  {!timeline.finished&&!timeline.error&&<Skeleton shape="conversation" label="Loading conversation" count={timeline.entries.length?1:2}/>}
  {!timeline.disposed&&<CommentComposer job={{...timeline.identity,source:'comments',thread:null,after:null,seq:0}} onPosted={message=>timeline.acceptPosted(message)}/>}
  {timeline.finished&&!timeline.entries.length&&!timeline.error&&<p className="empty">No discussion yet.</p>}
 </>;
}

// Only nearby rows mount Markdown and composers. Measured spacers preserve the
// scroll position as variable-height comments and nested threads leave view.
function useNearby(){
 const element=useRef<HTMLDivElement>(null),[near,setNear]=useState(typeof IntersectionObserver==='undefined');
 const height=useRef(240),inRange=useRef(typeof IntersectionObserver==='undefined'),pinned=useRef(false),rendered=useRef(near);
 rendered.current=near;
 function measure(){const measured=element.current?.getBoundingClientRect().height;if(measured&&measured>0)height.current=measured;}
 useLayoutEffect(()=>{
  if(!near)return;
  measure();
  if(typeof ResizeObserver==='undefined'||!element.current)return;
  const observer=new ResizeObserver(()=>measure());
  observer.observe(element.current);return()=>observer.disconnect();
 },[near]);
 useEffect(()=>{
  const node=element.current;if(!node||typeof IntersectionObserver==='undefined')return;
  const observer=new IntersectionObserver(entries=>{
   inRange.current=entries.some(e=>e.isIntersecting);
   const visible=inRange.current||pinned.current||node.contains(node.ownerDocument.activeElement);
   if(!visible&&rendered.current)measure();
   setNear(visible);
  },{rootMargin:'800px 0px'});
  observer.observe(node);return()=>observer.disconnect();
 },[]);
 return {element,near,height,pin:()=>{pinned.current=true;},focus:()=>setNear(true),blur:(event:React.FocusEvent<HTMLDivElement>)=>{
  if(!event.currentTarget.contains(event.relatedTarget)&&!inRange.current&&!pinned.current){measure();setNear(false);}
 }};
}
const TimelineRow=React.memo(function TimelineRow({entry,timeline,context}:{entry:TimelineEntry;timeline:Timeline;context?:PatchContext}){
 const {element,near,height,pin,focus,blur}=useNearby();
 return <div ref={element} onInput={pin} onFocus={focus} onBlur={blur} className="timeline-entry" aria-hidden={near?undefined:true} style={near?undefined:{height:height.current}}>
  {near&&(entry.kind==='message'?<TimelineBody entry={entry} timeline={timeline}/>:<ThreadDiscussion entry={entry} timeline={timeline} context={context}/>)}
 </div>;
});
function TimelineBody({entry,timeline}:{entry:TimelineMessage;timeline:Timeline}){
 useSyncExternalStore(timeline.subscribe,timeline.snapshot);
 const message=timeline.body(entry),[replying,setReplying]=useState(false);
 useEffect(()=>{void timeline.show(entry);},[entry,timeline]);
 return message?<><Message defaultExpanded={!timeline.collapsed.has(entry.id)} onExpandedChange={expanded=>{if(expanded)timeline.collapsed.delete(entry.id);else timeline.collapsed.add(entry.id);}} onReply={()=>setReplying(true)} continuous message={message} job={entry.job} generation={0} report={()=>timeline.reportError()}/>{replying&&<CommentComposer job={entry.job} thread={entry.job.source==='replies'?entry.job.thread:null} replyTo={message} onCancel={()=>setReplying(false)} onPosted={message=>{timeline.acceptPosted(message,entry.job.thread);setReplying(false);}}/>}</>:<article className="conversation-message"><header><strong>{entry.metadata.author??'Unknown author'}</strong><time dateTime={entry.time}>{conversationTime(entry.time)}</time></header>{timeline.error?<p className="reader-meta" role="status">Couldn’t load this comment. Refresh to try again.</p>:<Skeleton shape="text" label="Loading comment"/>}</article>;
}
function ThreadDiscussion({entry,timeline,context}:{entry:TimelineThread;timeline:Timeline;context?:PatchContext}){
 useSyncExternalStore(timeline.subscribe,timeline.snapshot);
 useEffect(()=>{void timeline.loadReplies(entry);},[entry,timeline]);
 const [expanded,setExpanded]=useState(()=>!timeline.collapsed.has(entry.id)),contentId=useId();
 const composer=useRef<HTMLDivElement>(null),[focusRequest,setFocusRequest]=useState(0);
 useLayoutEffect(()=>{if(focusRequest)composer.current?.querySelector('textarea')?.focus();},[focusRequest]);
 function reply(){timeline.collapsed.delete(entry.id);setExpanded(true);setFocusRequest(n=>n+1);}
 const thread=entry.thread,position=anchor(thread,context),replies=timeline.replies.get(entry.id);
 return <section className="timeline-thread" aria-label={`Discussion in ${thread.path}`}>
  <header className="timeline-thread-heading"><button type="button" className="thread-toggle" aria-expanded={expanded} aria-controls={contentId} aria-label={`${expanded?'Collapse':'Expand'} discussion in ${thread.path}`} onClick={()=>{if(expanded)timeline.collapsed.add(entry.id);else timeline.collapsed.delete(entry.id);setExpanded(!expanded);}}>{expanded?<ChevronDown size={14} strokeWidth={1.65}/>:<ChevronRight size={14} strokeWidth={1.65}/>}<strong>{thread.path}{thread.line?` · line ${thread.line}`:''}</strong></button><span>{thread.resolved?'Resolved':'Unresolved'}{thread.outdated?' · Outdated':''}</span>{!timeline.disposed&&<button type="button" className="reply-action" aria-label={`Reply to discussion in ${thread.path}`} aria-controls={contentId} onClick={reply}><Reply size={13} strokeWidth={1.65}/>Reply</button>}</header>
  <div id={contentId} hidden={!expanded}>
  {position.row!==null&&context&&<button className="timeline-diff-link" onClick={()=>context.jump(position.row!)}>Go to thread · line {thread.line}</button>}
  <div className="timeline-replies">{replies?.map(reply=><TimelineRow key={reply.id} entry={reply} timeline={timeline}/>)}{!replies&&<Skeleton shape="conversation" label="Loading replies" count={1}/>}{replies?.length===0&&<p className="reader-meta">No replies yet.</p>}</div>
  {!timeline.disposed&&<div ref={composer}><CommentComposer job={entry.job} thread={thread.id} onPosted={message=>timeline.acceptPosted(message,thread.id)}/></div>}
  {thread.url&&<button className="timeline-diff-link" onClick={()=>void command('open_conversation',{job:entry.job,id:thread.id}).catch(()=>timeline.reportError())}>View on GitHub</button>}
 </div></section>;
}
