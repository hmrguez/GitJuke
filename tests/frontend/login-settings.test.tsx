import React,{act} from 'react';
import {createRoot,type Root} from 'react-dom/client';
import {Window} from 'happy-dom';
import {beforeEach,afterEach,test,expect} from 'bun:test';
import {LoginSettings,type LoginStatus} from '../../src/features/preferences/LoginSettings';
const dom=new Window({url:'http://localhost'});
let host:HTMLElement,root:Root,calls:{name:string,args:any}[],current:LoginStatus,fail=false;
const settle=()=>act(async()=>{await new Promise(r=>setTimeout(r,0));});
beforeEach(()=>{
 Object.assign(globalThis,{window:dom,document:dom.document,HTMLElement:dom.HTMLElement,IS_REACT_ACT_ENVIRONMENT:true,isTauri:true});
 host=document.createElement('div');document.body.append(host);root=createRoot(host);calls=[];fail=false;
 current={desired:false,installed:true,registered:false,registrationValid:false,osDisabled:false,state:'off',message:'Launch at login is off.'};
 (dom as any).__TAURI_INTERNALS__={invoke:async(name:string,args:any)=>{calls.push({name,args});if(name==='set_launch_at_login'){current={...current,desired:args.enabled,registered:args.enabled,registrationValid:args.enabled};if(fail)throw Error('Registration failed; repair required');}return {...current};}};
});
afterEach(async()=>{await act(async()=>root.unmount());host.remove();});
async function render(){await act(async()=>root.render(<LoginSettings/>));await settle();}
async function click(text:string){const button=Array.from(host.querySelectorAll('button')).find(b=>b.textContent===text)!;await act(async()=>button.click());await settle();}
test('default-off inspection does not register or request notification permission',async()=>{
 await render();expect((host.querySelector('input') as HTMLInputElement).checked).toBe(false);expect(calls.map(c=>c.name)).toEqual(['login_status']);
});
test('explicit enable, OS-disabled repair and disable use only login commands',async()=>{
 current={...current,desired:true,registered:true,registrationValid:true,osDisabled:true,state:'osDisabled',message:'macOS disabled registration'};await render();
 expect(host.textContent).toContain('macOS has turned this off');await click('Fix launch at login');expect(calls.find(c=>c.name==='set_launch_at_login')?.args).toEqual({enabled:true,repair:true});
 await click('Disable launch at login');expect(calls.filter(c=>c.name==='set_launch_at_login').at(-1)?.args).toEqual({enabled:false,repair:false});expect((host.querySelector('input') as HTMLInputElement).checked).toBe(false);
 expect(calls.some(c=>c.name.includes('notification'))).toBe(false);
});
test('registration failure rereads actual saved preference and exposes explicit repair',async()=>{
 fail=true;await render();await act(async()=>host.querySelector<HTMLInputElement>('input')!.click());await settle();
 expect(host.querySelector('[role="alert"]')?.textContent).toContain('Couldn’t change launch at login.');expect((host.querySelector('input') as HTMLInputElement).checked).toBe(true);expect(host.textContent).toContain('Fix launch at login');
});
test('moved copy cannot enable or repair but can disable existing registration',async()=>{
 current={...current,installed:false,desired:true,registered:true,state:'installRequired',message:'Install GitJuke'};await render();
 expect(Array.from(host.querySelectorAll('button')).find(b=>b.textContent==='Fix launch at login')!.disabled).toBe(true);
 await click('Disable launch at login');expect(calls.filter(c=>c.name==='set_launch_at_login')).toHaveLength(1);
});
test('a delayed registration inspection cannot overwrite a newer explicit choice',async()=>{
 await render();let release!:(s:LoginStatus)=>void;const stale={...current};let reads=0;
 (dom as any).__TAURI_INTERNALS__={invoke:async(name:string,args:any)=>{
   calls.push({name,args});if(name==='set_launch_at_login'){current={...current,desired:true,registered:true,registrationValid:true};return {...current};}
   if(reads++===0)return new Promise<LoginStatus>(resolve=>{release=resolve;});return {...current};
 }};
 await act(async()=>window.dispatchEvent(new dom.Event('focus') as any));
 await act(async()=>host.querySelector<HTMLInputElement>('input')!.click());await settle();
 await act(async()=>release(stale));await settle();expect(host.querySelector<HTMLInputElement>('input')!.checked).toBe(true);
});
