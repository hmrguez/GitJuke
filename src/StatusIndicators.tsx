import React from 'react';
import {CircleDot,GitMerge,GitPullRequest,GitPullRequestClosed,GitPullRequestDraft} from 'lucide-react';
import type {PullRequest} from './api';

type PrStatus = Pick<PullRequest,'state'|'draft'>;

export function pullRequestState(pr:PrStatus){
  const state=pr.state.toLowerCase();
  if(state==='merged'||state==='closed')return state;
  if(state==='open')return pr.draft?'draft':'open';
  return 'unknown';
}

export function pullRequestLabel(pr:PrStatus){
  return pullRequestState(pr)==='draft'?'Draft':pr.state;
}

export function PullRequestIcon({pr,size=14}:{pr:PrStatus;size?:number}){
  const state=pullRequestState(pr);
  const Icon=state==='merged'?GitMerge:state==='closed'?GitPullRequestClosed:state==='draft'?GitPullRequestDraft:state==='open'?GitPullRequest:CircleDot;
  return <Icon size={size} aria-hidden="true"/>;
}

export function PullRequestBadge({pr}:{pr:PrStatus}){
  return <span className="open-badge" data-pr-state={pullRequestState(pr)}><PullRequestIcon pr={pr} size={12}/>{pullRequestLabel(pr)}</span>;
}
