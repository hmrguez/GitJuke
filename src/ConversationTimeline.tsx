import React,{useEffect,useId,useLayoutEffect,useRef,useState,useSyncExternalStore} from 'react';
import {command,type Overview} from './api';
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
  {!timeline.finished&&!timeline.error&&<p className="reader-meta" role="status">Loading conversation…</p>}
  {!timeline.disposed&&<CommentComposer job={{...timeline.identity,source:'comments',thread:null,after:null,seq:0}} onPosted={message=>timeline.acceptPosted(message)}/>}
  {timeline.finished&&!timeline.entries.length&&!timeline.error&&<p className="empty">No discussion yet.</p>}
 </>;
}

function useNearby(){
 const element=useRef<HTMLDivElement>(null),[near,setNear]=useState(typeof IntersectionObserver==='undefined'),height=useRef(100),pinned=useRef(false);
 useEffect(()=>{
  const node=element.current;if(!node||typeof IntersectionObserver==='undefined')return;
  const observer=new IntersectionObserver(entries=>{const visible=pinned.current||entries.some(e=>e.isIntersecting)||node.contains(document.activeElement);if(!visible)height.current=Math.max(100,node.getBoundingClientRect().height);setNear(visible);},{rootMargin:'800px'});
  observer.observe(node);return()=>observer.disconnect();
 },[]);
 return {element,near,height,pinned};
}
const TimelineRow=React.memo(function TimelineRow({entry,timeline,context}:{entry:TimelineEntry;timeline:Timeline;context?:PatchContext}){
 const {element,near,height,pinned}=useNearby();
 return <div ref={element} onInput={()=>{pinned.current=true;}} className="timeline-entry" style={near?undefined:{minHeight:height.current}} onFocus={()=>{if(entry.kind==='message')void timeline.show(entry);}}>
  {near?entry.kind==='message'?<TimelineBody entry={entry} timeline={timeline}/>:<ThreadDiscussion entry={entry} timeline={timeline} context={context}/>:<div className="timeline-placeholder"><strong>{entry.kind==='thread'?entry.thread.path:entry.metadata.author??'Unknown author'}</strong><time dateTime={entry.time}>{conversationTime(entry.time)}</time></div>}
 </div>;
});
function TimelineBody({entry,timeline}:{entry:TimelineMessage;timeline:Timeline}){
 useSyncExternalStore(timeline.subscribe,timeline.snapshot);
 const message=timeline.body(entry),[replying,setReplying]=useState(false);
 useEffect(()=>{void timeline.show(entry);},[entry,timeline]);
 return message?<><Message defaultExpanded={!timeline.collapsed.has(entry.id)} onExpandedChange={expanded=>{if(expanded)timeline.collapsed.delete(entry.id);else timeline.collapsed.add(entry.id);}} onReply={()=>setReplying(true)} continuous message={message} job={entry.job} generation={0} report={()=>timeline.reportError()}/>{replying&&<CommentComposer job={entry.job} thread={entry.job.source==='replies'?entry.job.thread:null} replyTo={message} onCancel={()=>setReplying(false)} onPosted={message=>{timeline.acceptPosted(message,entry.job.thread);setReplying(false);}}/>}</>:<article className="conversation-message"><header><strong>{entry.metadata.author??'Unknown author'}</strong><time dateTime={entry.time}>{conversationTime(entry.time)}</time></header><p className="reader-meta" role="status">{timeline.error?'Couldn’t load this comment. Refresh to try again.':'Loading comment…'}</p></article>;
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
  <div className="timeline-replies">{replies?.map(reply=><TimelineRow key={reply.id} entry={reply} timeline={timeline}/>)}{!replies&&<p className="reader-meta" role="status">Loading replies…</p>}{replies?.length===0&&<p className="reader-meta">No replies yet.</p>}</div>
  {!timeline.disposed&&<div ref={composer}><CommentComposer job={entry.job} thread={thread.id} onPosted={message=>timeline.acceptPosted(message,thread.id)}/></div>}
  {thread.url&&<button className="timeline-diff-link" onClick={()=>void command('open_conversation',{job:entry.job,id:thread.id}).catch(()=>timeline.reportError())}>View on GitHub</button>}
 </div></section>;
}
