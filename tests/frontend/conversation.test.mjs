import assert from 'node:assert/strict';
import {test} from 'node:test';
import {anchor,canMarkSeen,eligiblePage,retentionWithin,BODY_LIMIT,RETAINED_BYTES} from '../../src/features/conversation/conversation-model.ts';
import {parseDiff} from '../../src/features/changes/diff-model.ts';
const message=(body)=>({id:'I',kind:'APPROVED',author:null,time:'now',url:null,body,state:body===null?'unavailable':'complete',bytes:body===null?null:new TextEncoder().encode(body).length});
const page=(messages)=>({head:'a'.repeat(40),source:'comments',messages,threads:[],next:null,total:messages.length,complete:true});
test('inclusive UTF-8 body / page bytes / 50 messages and one-over rejection',()=>{
 assert.equal(eligiblePage(page([message('é'.repeat(BODY_LIMIT/2))])),true);
 assert.equal(eligiblePage(page([message('é'.repeat(BODY_LIMIT/2)+'x')])),false);
 const exact=Array.from({length:16},()=>message('x'.repeat(BODY_LIMIT)));
 assert.equal(exact.reduce((n,m)=>n+m.bytes,0),RETAINED_BYTES);assert.equal(eligiblePage(page(exact)),true);
 assert.equal(eligiblePage(page([...exact,message('x')])) ,false);
 assert.equal(eligiblePage(page(Array.from({length:50},()=>message('')))),true);
 assert.equal(eligiblePage(page(Array.from({length:51},()=>message('')))),false);
});
test('complete rendered bodies allow receipts while placeholders do not',()=>{
 assert.equal(canMarkSeen(message('')),true);assert.equal(canMarkSeen(message('long')),true);
 assert.equal(canMarkSeen(message(null)),false);assert.equal(canMarkSeen({...message(null),state:'oversized'}),false);
});
const thread={id:'T',path:'file.txt',side:'RIGHT',line:5,startLine:null,startSide:null,originalLine:3,originalStartLine:null,commit:'a'.repeat(40),originalCommit:'c'.repeat(40),outdated:false,resolved:false,url:null,time:'now'};
const context={head:'a'.repeat(40),base:'b'.repeat(40),file:{path:'file.txt',previousPath:'old.txt'},model:parseDiff('@@ -4,2 +4,2 @@\n same\n-old\n+new'),jump:()=>{}};
test('anchors require exact file, revision, side and known unique supplied coordinates',()=>{
 assert.equal(anchor(thread,context).row,3);
 for(const patch of [{path:'old.txt'},{commit:'c'.repeat(40)},{side:'LEFT'},{line:99},{line:null},{side:'UNKNOWN'},{outdated:true},{startLine:1},{startLine:4,startSide:null},{startLine:4,startSide:'LEFT'}])assert.equal(anchor({...thread,...patch},context).row,null);
 assert.equal(anchor({...thread,side:'LEFT',commit:context.base},context).row,2);
 assert.equal(anchor(thread).row,null);
 const dup={...context,model:{...context.model,rows:[...context.model.rows,context.model.rows[3]]}};assert.equal(anchor(thread,dup).row,null);
});
test('combined transfer message ceiling is inclusive at 100 and rejects 101',()=>{
 assert.equal(retentionWithin(Array.from({length:100},()=>message(''))),true);
 assert.equal(retentionWithin(Array.from({length:101},()=>message(''))),false);
});

test('LEFT and RIGHT ranges across semantic page boundaries require every unique coordinate',()=>{
 const source='@@ -1,201 +1,201 @@\n'+Array.from({length:201},(_,i)=>`-old ${i+1}\n+new ${i+1}`).join('\n');
 const c={...context,file:{path:'file.txt'},model:parseDiff(source)};
 const right={...thread,line:201,startLine:199,startSide:'RIGHT'};
 assert.equal(anchor(right,c).row,402);
 assert.equal(anchor({...right,side:'LEFT',startSide:'LEFT',commit:c.base},c).row,401);
 for(const change of [{line:-1},{line:0},{startLine:-1},{startLine:202},{startSide:'LEFT'},{commit:c.base},{path:'other.txt'},{outdated:true}])assert.equal(anchor({...right,...change},c).row,null);
 const pages=[];for(let n=0;n<c.model.rows.length;n+=200)pages.push(...c.model.rows.slice(n,n+200));assert.deepEqual(pages,c.model.rows);
});
