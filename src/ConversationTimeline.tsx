import React,{useEffect,useRef,useState,useSyncExternalStore} from 'react';
import {command,type Overview} from './api';
import {anchor,conversationTime,type PatchContext} from './conversation-model';
import {ConversationTimeline as Timeline,type TimelineEntry,type TimelineMessage,type TimelineThread} from './conversation-timeline';
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
  {timeline.finished&&!timeline.entries.length&&!timeline.error&&<p className="empty">No discussion yet.</p>}
 </>;
}

function useNearby(){
 const element=useRef<HTMLDivElement>(null),[near,setNear]=useState(typeof IntersectionObserver==='undefined'),height=useRef(100);
 useEffect(()=>{
  const node=element.current;if(!node||typeof IntersectionObserver==='undefined')return;
  const observer=new IntersectionObserver(entries=>{const visible=entries.some(e=>e.isIntersecting)||node.contains(document.activeElement);if(!visible)height.current=Math.max(100,node.getBoundingClientRect().height);setNear(visible);},{rootMargin:'800px'});
  observer.observe(node);return()=>observer.disconnect();
 },[]);
 return {element,near,height};
}
const TimelineRow=React.memo(function TimelineRow({entry,timeline,context}:{entry:TimelineEntry;timeline:Timeline;context?:PatchContext}){
 const {element,near,height}=useNearby();
 return <div ref={element} className="timeline-entry" style={near?undefined:{minHeight:height.current}} onFocus={()=>{if(entry.kind==='message')void timeline.show(entry);}}>
  {near?entry.kind==='message'?<TimelineBody entry={entry} timeline={timeline}/>:<ThreadDiscussion entry={entry} timeline={timeline} context={context}/>:<div className="timeline-placeholder"><strong>{entry.kind==='thread'?entry.thread.path:entry.metadata.author??'Unknown author'}</strong><time dateTime={entry.time}>{conversationTime(entry.time)}</time></div>}
 </div>;
});
function TimelineBody({entry,timeline}:{entry:TimelineMessage;timeline:Timeline}){
 useSyncExternalStore(timeline.subscribe,timeline.snapshot);
 const message=timeline.body(entry);
 useEffect(()=>{void timeline.show(entry);},[entry,timeline]);
 return message?<Message continuous message={message} job={entry.job} generation={0} report={()=>timeline.reportError()}/>:<article className="conversation-message"><header><strong>{entry.metadata.author??'Unknown author'}</strong><time dateTime={entry.time}>{conversationTime(entry.time)}</time></header><p className="reader-meta" role="status">{timeline.error?'Couldn’t load this comment. Refresh to try again.':'Loading comment…'}</p></article>;
}
function ThreadDiscussion({entry,timeline,context}:{entry:TimelineThread;timeline:Timeline;context?:PatchContext}){
 useSyncExternalStore(timeline.subscribe,timeline.snapshot);
 useEffect(()=>{void timeline.loadReplies(entry);},[entry,timeline]);
 const thread=entry.thread,position=anchor(thread,context),replies=timeline.replies.get(entry.id);
 return <section className="timeline-thread" aria-label={`Discussion in ${thread.path}`}>
  <header className="timeline-thread-heading"><strong>{thread.path}{thread.line?` · line ${thread.line}`:''}</strong><span>{thread.resolved?'Resolved':'Unresolved'}{thread.outdated?' · Outdated':''}</span></header>
  {position.row!==null&&context&&<button className="timeline-diff-link" onClick={()=>context.jump(position.row!)}>Go to thread · line {thread.line}</button>}
  <div className="timeline-replies">{replies?.map(reply=><TimelineRow key={reply.id} entry={reply} timeline={timeline}/>)}{!replies&&<p className="reader-meta" role="status">Loading replies…</p>}{replies?.length===0&&<p className="reader-meta">No replies yet.</p>}</div>
  {thread.url&&<button className="timeline-diff-link" onClick={()=>void command('open_conversation',{job:entry.job,id:thread.id}).catch(()=>timeline.reportError())}>View on GitHub</button>}
 </section>;
}
