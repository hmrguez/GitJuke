import React from 'react';
import {renderToStaticMarkup} from 'react-dom/server';
import {expect,test} from 'bun:test';
import {CheckReader} from '../src/Checks';
import type {CheckPage} from '../src/api';

const head='a'.repeat(40);
const checks:CheckPage={pr:null,error:null,review:'CHANGES_REQUESTED',head,revision:7,nextOffset:null,commits:[{resource:head,complete:true,fetchedAt:123,error:null}],items:[{id:'CR_1',name:'Unit tests',kind:'CheckRun',status:'COMPLETED',conclusion:'SUCCESS',commit:head,runId:987,attempt:3,url:'https://github.com/a/b/actions/runs/987/job/1',suiteId:null,suiteStatus:null,suiteConclusion:null}]};
const render=(value:CheckPage|null)=>renderToStaticMarkup(<CheckReader repo="a/b" number={1} checks={value} revision={7} offset={0} setOffset={()=>{}}/>);
test('checks retain review decisions, names, outcomes and useful actions without diagnostics',()=>{
 const html=render(checks);expect(html).not.toContain('<h2');
 for(const text of ['Changes requested','Unit tests','Passed','Show log','View details'])expect(html).toContain(text);
 for(const text of [head,'987','attempt','CHANGES_REQUESTED','cached contexts','Legacy status','loaded','Previous checks'])expect(html).not.toContain(text);
});
test('missing or incomplete checks never imply a passing result',()=>{
 expect(render(null)).toContain('Loading checks…');
 expect(render({...checks,items:[],commits:[]})).toContain('Checks aren’t available yet.');
 expect(render({...checks,items:[]})).toContain('No checks to show.');
});
test('failed refresh preserves results and gives a plain-language stale warning',()=>{
 const html=render({...checks,error:'internal resource revision mismatch'});
 expect(html).toContain('Results may be out of date.');expect(html).toContain('Passed');
 expect(html).not.toContain('internal resource revision mismatch');
});
