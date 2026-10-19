import React,{act} from 'react';import {createRoot,type Root} from 'react-dom/client';import {Window} from 'happy-dom';import {beforeEach,afterEach,test,expect} from 'bun:test';import {NotificationControls} from '../../src/features/notifications/NotificationControls';
const dom=new Window();Object.assign(globalThis,{window:dom,document:dom.document,HTMLElement:dom.HTMLElement,IS_REACT_ACT_ENVIRONMENT:true});
let root:Root,host:HTMLElement,calls:{name:string;args:any}[];
const settings={enabled:false,onboarded:false,previews:false,categories:[true,true,true,true,true],permission:'notDetermined' as const};
beforeEach(()=>{calls=[];host=document.createElement('div');document.body.append(host);root=createRoot(host);});afterEach(async()=>{await act(async()=>root.unmount());host.remove();});
const action=async(name:string,args?:Record<string,unknown>)=>{calls.push({name,args});};
const render=async(node:React.ReactNode)=>{await act(async()=>root.render(node));};
const click=async(name:string)=>{await act(async()=>Array.from(host.querySelectorAll('button')).find(b=>b.textContent===name)!.click());};
test('onboarding requests nothing until explicit Enable; Skip persists a disabled choice',async()=>{await render(<NotificationControls settings={settings} action={action} onboarding/>);expect(calls).toHaveLength(0);expect(host.textContent).toContain('hide PR details by default');await click('Skip for now');expect(calls[0].name).toBe('set_alert_choices');expect(calls[0].args.choices.enabled).toBe(false);await click('Enable notifications');expect(calls[1].name).toBe('enable_notifications');});
test('denial offers settings without requesting permission again; toggles preserve opt-in',async()=>{await render(<NotificationControls settings={{...settings,enabled:true,permission:'denied'}} action={action}/>);expect(calls).toHaveLength(0);await click('Open System Settings');expect(calls[0].name).toBe('notification_settings');const inputs=host.querySelectorAll<HTMLInputElement>('input');await act(async()=>inputs[0].click());expect(calls[1].args.choices.categories).toEqual([false,true,true,true,true]);expect(calls[1].args.choices.enabled).toBe(true);expect(calls.every(c=>c.name!=='enable_notifications')).toBe(true);});
test('message previews use the explicit checkbox and explain private content',async()=>{await render(<NotificationControls settings={settings} action={action}/>);const inputs=host.querySelectorAll<HTMLInputElement>('input');expect(inputs[5].checked).toBe(false);await act(async()=>inputs[5].click());expect(calls[0].args.choices.previews).toBe(true);expect(calls[0].args.choices.enabled).toBe(false);expect(host.textContent).toContain('Show PR details and message previews');expect(host.textContent).toContain('a shortened message, including private pull requests');expect(host.textContent).toContain('Logs are never included');});
test('permission labels and denied settings action accept the actual camelCase IPC states',async()=>{
 for(const [permission,label] of [['unknown','Checking notification settings…'],['notDetermined','Notifications are off.'],['denied','Notifications are blocked in System Settings.'],['granted','Notifications are off.']] as const){
  await render(<NotificationControls settings={{...settings,permission}} action={action}/>);
  expect(host.textContent).toContain(label);
  expect([...host.querySelectorAll('button')].some(b=>b.textContent==='Open System Settings')).toBe(permission==='denied');
 }
 expect(calls).toHaveLength(0);
});
