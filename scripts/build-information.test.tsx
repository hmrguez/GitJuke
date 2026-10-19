import React,{act} from 'react';
import {createRoot,type Root} from 'react-dom/client';
import {Window} from 'happy-dom';
import {beforeEach,afterEach,expect,test} from 'bun:test';
import {readFileSync} from 'node:fs';
import {BuildInformation} from '../src/BuildInformation';
import {Startup} from '../src/Startup';
import type {BuildInfo} from '../src/api';

// An optional captured real Rust IPC response runs the same rendering assertions.
const info:BuildInfo=process.env.GITJUKE_BUILD_INFO_FIXTURE?JSON.parse(readFileSync(process.env.GITJUKE_BUILD_INFO_FIXTURE,'utf8')):{version:'9.8.7',buildId:'ci.123.2',revision:'a'.repeat(40),sourceState:'modified',target:'aarch64-apple-darwin',profile:'release',schemaVersion:16};
let dom:Window,host:HTMLElement,root:Root,calls:string[],read:()=>Promise<BuildInfo>;
const settle=()=>act(async()=>{await new Promise(resolve=>setTimeout(resolve,0));});
beforeEach(()=>{
 dom=new Window();Object.assign(globalThis,{window:dom,document:dom.document,isTauri:true,IS_REACT_ACT_ENVIRONMENT:true});
 host=document.createElement('div');document.body.append(host);root=createRoot(host);calls=[];read=async()=>info;
 (dom as any).__TAURI_INTERNALS__={invoke:async(name:string)=>{calls.push(name);if(name==='build_info')return read();if(name==='startup_status')return {ready:false,databaseState:'newer',databasePath:'/disposable/data'};throw new Error('Unexpected command');}};
});
afterEach(async()=>{await act(async()=>root.unmount());host.remove();});

test('About renders the binary version, build and format contract without remote or permission commands',async()=>{
 await act(async()=>root.render(<BuildInformation/>));await settle();
 for(const value of [info.version,info.buildId,info.revision,info.target,info.profile,String(info.schemaVersion)])expect(host.textContent).toContain(value);
 expect(host.textContent?.includes('modified sources')).toBe(info.sourceState==='modified');
 expect(calls).toEqual(['build_info']);
 expect(host.textContent).not.toContain('signed');
});

test('source archives show unavailable rather than inventing a source revision',async()=>{
 read=async()=>({...info,revision:'unavailable',sourceState:'unavailable'});
 await act(async()=>root.render(<BuildInformation/>));await settle();
 expect(host.textContent).toContain('Unavailable');expect(host.textContent).not.toContain('modified sources');
});

test('build information failures can be retried independently of startup or monitoring',async()=>{
 read=async()=>{throw new Error('private internals');};
 await act(async()=>root.render(<BuildInformation/>));await settle();
 expect(host.textContent).toContain('couldn’t be loaded');expect(host.textContent).not.toContain('private internals');
 read=async()=>info;
 await act(async()=>host.querySelector('button')!.click());await settle();
 expect(host.textContent).toContain(info.buildId);expect(calls).toEqual(['build_info','build_info']);
});

test('newer-database recovery exposes the running binary while keeping the reader unmounted',async()=>{
 await act(async()=>root.render(<Startup><p>private reader</p></Startup>));await settle();
 expect(host.textContent).toContain('newer version');expect(host.textContent).toContain(info.buildId);
 expect(host.textContent).not.toContain('private reader');expect(calls).toEqual(['startup_status','build_info']);
});
