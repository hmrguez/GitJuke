export type * from './types';
import { invoke, isTauri } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
export function command<T>(name:string,args?:Record<string,unknown>):Promise<T>{
  if(!isTauri())return Promise.reject(new Error('Open the native GitJuke app with npm run tauri dev. This browser view has no runtime or fixture fallback.'));
  return invoke<T>(name,args);
}
export const revisions=(handler:()=>void)=>listen<number>('cache-revision',handler);
