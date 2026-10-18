import { invoke, isTauri } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
export type Theme = {mode:'dark'|'light';canvas:string;surface:string;accent:string;ink:string};
export type PullRequest = {id:string;repoId:number|null;repo:string;number:number;title:string;url:string;state:string;draft:boolean;author:string|null;updatedAt:string};
export type Stack = {state:'unknown';reason:string}|{state:'none'}|{state:'published';number:number;trunk:string;members:{number:number;title:string;url:string}[]};
export type Overview = {id:string;repo:string;number:number;title:string;url:string;body:string|null;bodyLimited:boolean;head:string;base:string;state:string;draft:boolean;author:string|null;stack:Stack};
export type Snapshot<T> = {data:T;fetchedAt:number|null;complete:boolean;error:string|null;revision:number};
export type Page = {items:PullRequest[];nextOffset:number|null;fetchedAt:number|null;complete:boolean;error:string|null;revision:number};
export type ResourceHealth={resource:string;fetchedAt:number|null;complete:boolean;error:string|null};
export type Budget={resource:string;remaining:number;resetAt:number;blockedUntil:number;requests:number;payloadBytes:number;cost:number};
export type WatchRow={id:string;repo:string;number:number;reasons:string[];head:string|null;merge:string|null;refreshedAt:number|null;dueAt:number;error:string|null};
export type WatchPage={items:WatchRow[];nextOffset:number|null;revision:number};
export type CheckContext={id:string;kind:string;name:string;status:string;conclusion:string|null;url:string|null;commit:string;suiteId:string|null;suiteStatus:string|null;suiteConclusion:string|null;runId:number|null;attempt:number|null};
export type CheckPage={pr:PullRequest|null;error:string|null;review:string|null;items:CheckContext[];nextOffset:number|null;head:string|null;commits:ResourceHealth[];revision:number};
export type Status = {settings:{paused:boolean;theme:Theme;ghPath:string|null;configDir:string|null};identity:{id:number;login:string}|null;locked:boolean;health:string;storageStopped:boolean;storage:{databaseBytes:number;walBytes:number;sharedMemoryBytes:number;journalBytes:number;managedBytes:number;disposableBytes:number;contentLimitBytes:number;warningAtBytes:number;warning:boolean};ghVersion:string|null;ghPath:string|null;configDir:string;ignoredOverrides:string[];revision:number;queued:number;busy:boolean;nextRemoteAt:number;discoveryMore:boolean;watchCount:number;delayedWatches:number;discovery:ResourceHealth[];budgets:Budget[];selected:[string,number]|null};
export function command<T>(name:string,args?:Record<string,unknown>):Promise<T>{
  if(!isTauri())return Promise.reject(new Error('Open the native GitJuke app with npm run tauri dev. This browser view has no runtime or fixture fallback.'));
  return invoke<T>(name,args);
}
export const revisions=(handler:()=>void)=>listen<number>('cache-revision',handler);
export const presets:Record<'dark'|'light',Theme> = {
  dark:{mode:'dark',canvas:'#121512',surface:'#1c211b',accent:'#b8d68a',ink:'#e7ecdf'},
  light:{mode:'light',canvas:'#e9ebe5',surface:'#fcfdf8',accent:'#6c8548',ink:'#293126'},
};

export type ActivityRecord={id:string;repo:string;number:number;kind:string;occurred:number;actor:string|null;url:string;text:string|null;eligible:boolean;reason:string;seen:boolean;head:string|null};
export type ActivityPage={items:ActivityRecord[];nextOffset:number|null;revision:number;baseline:boolean;recovery:string|null;pending:number;error:string|null;nextPollAt:number};
