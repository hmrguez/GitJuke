import React,{useId,useRef,useState} from 'react';
import {Send} from 'lucide-react';
import {command,type ConversationJob,type ConversationMessage} from '../../lib/ipc';

export function CommentComposer({job,thread=null,replyTo,onPosted,onCancel}:{job:ConversationJob;thread?:string|null;replyTo?:ConversationMessage;onPosted:(message:ConversationMessage)=>void;onCancel?:()=>void}){
 const id=useId(),[body,setBody]=useState(''),[sending,setSending]=useState(false),[error,setError]=useState(''),[posted,setPosted]=useState(false),busy=useRef(false);
 const reply=!!thread||!!replyTo;
 async function submit(event:React.FormEvent){
  event.preventDefault();if(busy.current||!body.trim())return;
  busy.current=true;setSending(true);setError('');setPosted(false);
  const text=replyTo&&!thread?`> @${replyTo.author??'unknown'} wrote:\n${(replyTo.body??'').split('\n').map(line=>`> ${line}`).join('\n')}\n\n${body}`:body;
  try{const message=await command<ConversationMessage>('post_conversation',{job,thread,body:text});setBody('');setPosted(true);onPosted(message);}
  catch(e){setError(String(e).replace(/^Error: /,''));}
  finally{busy.current=false;setSending(false);}
 }
 return <form className="comment-composer" onSubmit={submit} onKeyDown={event=>{if((event.metaKey||event.ctrlKey)&&event.key==='Enter'&&!event.nativeEvent.isComposing){event.preventDefault();event.currentTarget.requestSubmit();}}} aria-label={reply?'Write a reply':'Write a comment'}>
  <label htmlFor={id}>{thread?'Reply to this thread':replyTo?`Quote reply to ${replyTo.author??'comment'}`:'Add to the discussion'}</label>
  <textarea id={id} value={body} onInput={event=>{setBody(event.currentTarget.value);setPosted(false);}} placeholder={reply?'Write a reply…':'Write a comment…'} rows={3} maxLength={65536} disabled={sending} autoFocus={!!replyTo}/>
  {error&&<p className="state-banner" role="alert">{error}</p>}
  <div className="comment-composer-actions"><span className="reader-meta" role="status">{posted?(thread?'Reply posted.':'Comment posted.'):replyTo&&!thread?'Posts a quoted comment to the PR conversation':'Markdown supported · ⌘/Ctrl + Enter to send'}</span>{onCancel&&<button type="button" disabled={sending} onClick={onCancel}>Cancel</button>}<button type="submit" className="comment-submit" disabled={sending||!body.trim()}><Send size={13} strokeWidth={1.65}/>{sending?'Posting…':thread?'Reply':replyTo?'Post quoted comment':'Comment'}</button></div>
 </form>;
}
