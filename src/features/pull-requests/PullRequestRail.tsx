import React from 'react';
import {GitBranch} from 'lucide-react';
import type {PullRequest,PullRequestStack} from '../../lib/ipc';
import {PullRequestIcon,pullRequestLabel,pullRequestState} from './StatusIndicators';
import './pull-request-rail.css';

type PublishedStack=PullRequestStack['stack'];
type RailGroup={key:string;repo:string;stack?:PublishedStack;items:PullRequest[]};
const identity=(pr:{repo:string;number:number})=>`${pr.repo}#${pr.number}`;

// Only group PRs present in this feed. A stack must never add filtered-out results
// or imply relationships from matching branch names or titles.
export function groupPullRequests(items:PullRequest[],stacks:{repo:string;stack:PublishedStack}[]):RailGroup[]{
  const groups:RailGroup[]=[],seen=new Set<string>();
  const updated=(pr:PullRequest)=>Date.parse(pr.updatedAt)||0;
  const sorted=[...items].sort((a,b)=>updated(b)-updated(a));
  const membership=new Map<string,PublishedStack>();
  for(const {repo,stack} of stacks)for(const member of stack.members)membership.set(identity({repo,number:member.number}),stack);
  for(const pr of sorted){
    const stack=membership.get(identity(pr));
    const key=stack?`${pr.repo}:stack:${stack.number}`:identity(pr);
    if(seen.has(key))continue;
    seen.add(key);
    const members=stack?new Set(stack.members.map(member=>member.number)):null;
    const grouped=members?sorted.filter(item=>item.repo===pr.repo&&members.has(item.number)):[pr];
    groups.push({key,repo:pr.repo,stack,items:grouped});
  }
  return groups;
}

export function PullRequestRail({items,stacks=[],selected,search='',select}:{
  items:PullRequest[];stacks?:PullRequestStack[];selected?:{repo:string;number:number}|null;search?:string;
  select:(repo:string,number:number,origin:string)=>void;
}){
  const visible=items.filter(pr=>`${pr.repo} ${pr.number} ${pr.title}`.toLowerCase().includes(search.toLowerCase()));
  const groups=groupPullRequests(visible,stacks);
  return <div className="pr-list pr-rail">{groups.map(group=><section className={group.stack?'rail-stack':'rail-single'} key={group.key} aria-label={group.stack?`${group.repo}, stack #${group.stack.number}`:undefined}>
    {group.stack&&<div className="rail-stack-heading"><GitBranch size={12}/><span title={group.repo}>{group.repo}</span><span className="rail-stack-number">Stack #{group.stack.number}</span></div>}
    {group.stack&&<div className="rail-trunk" title={`Base branch: ${group.stack.trunk}`}><span aria-hidden="true" className="rail-root"/><span>{group.stack.trunk}</span><span className="rail-stack-count">{group.items.length} PR{group.items.length===1?'':'s'}</span></div>}
    <ol role="list" className={group.stack?'rail-branches':'rail-items'}>{group.items.map(pr=><li key={pr.id}>
      <button id={`pr-${pr.id}`} className={`stack-node rail-card ${selected&&identity(selected)===identity(pr)?'active':''}`} data-pr-state={pullRequestState(pr)} title={pr.title} aria-current={selected&&identity(selected)===identity(pr)?'page':undefined} onClick={()=>select(pr.repo,pr.number,`pr-${pr.id}`)}>
        <span className="rail-card-icon" title={pullRequestLabel(pr)}><PullRequestIcon pr={pr} size={14}/></span>
        <strong className="rail-card-title">{pr.title}</strong>
        <span className="rail-card-meta"><span className="rail-pr-number">#{pr.number}</span>{!group.stack&&<span className="rail-repo" title={pr.repo}>{pr.repo}</span>}<span className="rail-author" title={pr.author??'Unavailable author'}>{pr.author??'Unavailable author'}</span><span className="rail-state">{pullRequestLabel(pr)}</span></span>
      </button>
    </li>)}</ol>
  </section>)}</div>;
}
