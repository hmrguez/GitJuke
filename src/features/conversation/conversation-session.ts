import {command,type ConversationJob} from '../../lib/ipc';
import {ConversationTimeline} from './conversation-timeline';

// The selected PR owns loading and retained data. Views only borrow this session.
export class ConversationSession {
 private current?:{key:string;timeline:ConversationTimeline;ready:Promise<ConversationTimeline>};
 prepare(identity:Pick<ConversationJob,'repo'|'number'|'head'>,revision:number){
  this.begin(identity,revision);
  return this.current!.ready;
 }
 // Expose the shared session immediately; its traversal continues in the background.
 begin(identity:Pick<ConversationJob,'repo'|'number'|'head'>,revision:number){
  const key=JSON.stringify([identity.repo,identity.number,identity.head]);
  if(this.current?.key===key&&!this.current.timeline.disposed)return this.current.timeline;
  this.clear();
  const timeline=new ConversationTimeline(identity,revision);
  const ready=timeline.start().then(()=>timeline);
  this.current={key,timeline,ready};
  return timeline;
 }
 clear(){
  if(!this.current)return;
  this.current.timeline.close();this.current=undefined;
  void command('cancel_conversation').catch(()=>{});
 }
}
