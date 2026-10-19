import React from 'react';
import {test,expect} from 'bun:test';
import {renderToStaticMarkup} from 'react-dom/server';
import {Message} from '../../src/features/conversation/Conversation';
import {CONTENT_NODES} from '../../src/features/conversation/conversation-model';
test('50 maximum safe plain-text messages stay below content DOM budget without resource or executable elements',()=>{
 const body='<script>alert(1)</script>![private](https://evil.test/a)\n'.repeat(2000).slice(0,131072);
 const job={repo:'a/b',number:1,head:'a'.repeat(40),source:'comments' as const,thread:null,after:null,seq:0};
 const html=renderToStaticMarkup(<>{Array.from({length:50},(_,i)=><Message key={i} generation={1} job={job} report={()=>{}} message={{id:String(i),kind:'comment',author:null,time:'now',url:null,body,state:'complete',bytes:131072,digest:'digest'}}/>)}</>);
 expect(html).not.toMatch(/<(?:script|img|iframe|object|embed|video|audio)\b/);
 expect(html).toContain('&lt;script&gt;');
 // Count both element and text/comment nodes in the static result, conservatively.
 const nodes=(html.match(/<[^>]*>|[^<]+/g)??[]).filter(n=>!n.startsWith('</')).length;
 expect(nodes).toBeLessThanOrEqual(CONTENT_NODES);
});

test('ordinary messages have no seen control and destination placeholders stay labeled',()=>{
 const job={repo:'a/b',number:1,head:'a'.repeat(40),source:'destination' as const,thread:'target',after:null,seq:0};
 const html=renderToStaticMarkup(<Message generation={1} job={job} explicitlyOpened report={()=>{}} message={{id:'target',kind:'comment',author:null,time:'now',url:null,body:null,state:'oversized',bytes:131073,digest:null}}/>);
 expect(html).toContain('This comment is too long to preview. View it on GitHub.');expect(html).not.toContain('Open Activity destination');
});

test('review messages keep decisions and content without transfer metadata',()=>{
 const job={repo:'a/b',number:1,head:'a'.repeat(40),source:'reviews' as const,thread:null,after:null,seq:0};
 const html=renderToStaticMarkup(<Message generation={1} job={job} report={()=>{}} message={{id:'review',kind:'CHANGES_REQUESTED',author:'reviewer',time:'today',url:null,body:'Please cover the empty state.',state:'complete',bytes:29,digest:'digest'}}/>);
 for(const text of ['reviewer','Requested changes','today','Please cover the empty state.'])expect(html).toContain(text);
 for(const text of ['UTF-8','bytes','CHANGES_REQUESTED','unseen','explicit segment'])expect(html).not.toContain(text);
});

test('timeline messages show long comments in full without part navigation',()=>{
 const job={repo:'a/b',number:1,head:'a'.repeat(40),source:'comments' as const,thread:null,after:null,seq:0};
 const body='Long comment '.repeat(6000)+'end of comment';
 const html=renderToStaticMarkup(<Message continuous generation={1} job={job} report={()=>{}} message={{id:'long',kind:'comments',author:'reviewer',time:'today',url:null,body,state:'complete',bytes:body.length,digest:'long'}}/>);
 expect(html).toContain('end of comment');expect(html).not.toContain('Next part');expect(html).not.toContain('Previous part');
});
