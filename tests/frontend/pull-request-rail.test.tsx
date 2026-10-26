import React,{act} from 'react';
import {createRoot} from 'react-dom/client';
import {Window} from 'happy-dom';
import {expect,test} from 'bun:test';
import {groupPullRequests,PullRequestRail} from '../../src/features/pull-requests/PullRequestRail';
import type {PullRequest,Stack} from '../../src/lib/ipc';

type Published=Extract<Stack,{state:'published'}>;
const pr=(repo:string,number:number):PullRequest=>({id:`${repo}-${number}`,repo,repoId:null,number,title:`Change ${number}`,url:'',state:'open',draft:false,author:'reader',updatedAt:''});
const stack=(number:number,members:number[]):Published=>({state:'published',number,trunk:'main',members:members.map(number=>({number,title:`Change ${number}`,url:''}))});

test('rail groups multiple stacks per repository by recency without mixing repositories or adding absent PRs',()=>{
  const items=[pr('a/b',3),pr('a/b',9),pr('c/d',2),pr('a/b',1),pr('a/b',8),pr('a/b',20)];
  const groups=groupPullRequests(items,[{repo:'a/b',stack:stack(1,[1,2,3])},{repo:'a/b',stack:stack(2,[8,9])},{repo:'c/d',stack:stack(1,[2])}]);
  expect(groups.map(group=>[group.key,group.items.map(p=>p.number)])).toEqual([
    ['a/b:stack:1',[3,1]],['a/b:stack:2',[9,8]],['c/d:stack:1',[2]],['a/b#20',[20]],
  ]);
});

test('rail renders bundled stacks immediately without secondary requests, filters and preserves selection',async()=>{
  const dom=new Window({url:'http://localhost'});
  Object.assign(globalThis,{window:dom,document:dom.document,HTMLElement:dom.HTMLElement,IS_REACT_ACT_ENVIRONMENT:true,isTauri:true});
  const calls:string[]=[];
  (dom as any).__TAURI_INTERNALS__={invoke:async(name:string,args:{number:number})=>{
    calls.push(name);throw new Error('The rail must not make a secondary request');
  }};
  const host=document.createElement('div');document.body.append(host);const root=createRoot(host);
  const items=[pr('a/b',3),pr('a/b',1)],stacks=[{repo:'a/b',stack:stack(7,[1,2,3])}];let opened:unknown;
  try{
    await act(async()=>{root.render(<PullRequestRail items={items} stacks={stacks} selected={{repo:'a/b',number:3}} select={(...args)=>{opened=args;}}/>);});
    expect(host.querySelectorAll('.rail-stack')).toHaveLength(1);
    expect([...host.querySelectorAll('.rail-pr-number')].map(el=>el.textContent)).toEqual(['#3','#1']);
    const selected=host.querySelector<HTMLButtonElement>('[aria-current=page]')!;
    await act(async()=>selected.click());expect(opened).toEqual(['a/b',3,'pr-a/b-3']);
    await act(async()=>root.render(<PullRequestRail items={items} stacks={stacks} search="Change 3" select={()=>{}}/>));
    expect(host.querySelectorAll('.rail-card')).toHaveLength(1);
    expect(host.textContent).not.toContain('Change 1');
    expect(calls).toEqual([]);
  }finally{await act(async()=>root.unmount());host.remove();}
});


test('rail orders stacks and standalone PRs by newest member update, and members by recency',()=>{
  const dated=(number:number,day:number)=>({...pr('a/b',number),updatedAt:`2026-10-${day}T00:00:00Z`});
  const items=[dated(1,20),dated(2,24),dated(3,22),dated(4,25),dated(5,23)];
  const before=items.map(item=>item.number);
  const groups=groupPullRequests(items,[{repo:'a/b',stack:stack(1,[1,2])},{repo:'a/b',stack:stack(2,[3,5])}]);
  expect(groups.map(group=>[group.key,group.items.map(item=>item.number)])).toEqual([
    ['a/b#4',[4]],['a/b:stack:1',[2,1]],['a/b:stack:2',[5,3]],
  ]);
  expect(items.map(item=>item.number)).toEqual(before);
});
