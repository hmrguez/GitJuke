import React,{act} from 'react';
import {createRoot,type Root} from 'react-dom/client';
import {Window} from 'happy-dom';
import {beforeEach,afterEach,expect,test} from 'bun:test';
import {Sidebar,useSidebarState} from '../src/Sidebar';

let dom:Window,host:HTMLElement,root:Root;
beforeEach(()=>{
  dom=new Window({url:'http://localhost'});
  Object.assign(globalThis,{window:dom,document:dom.document,IS_REACT_ACT_ENVIRONMENT:true});
  host=document.createElement('div');document.body.append(host);root=createRoot(host);
});
afterEach(async()=>{await act(async()=>root.unmount());host.remove();});
const key='gitjuke.sidebar-collapsed.v1';
function Shell(){const sidebar=useSidebarState();return <Sidebar {...sidebar}><nav id="sidebar-navigation"><button aria-label="My PRs">My PRs</button></nav></Sidebar>;}
async function render(){await act(async()=>root.render(<Shell/>));}

test('collapse retains navigation and keyboard focus, and persists across remounts',async()=>{
  await render();
  const toggle=host.querySelector<HTMLButtonElement>('.sidebar-toggle')!;
  expect(toggle.getAttribute('aria-expanded')).toBe('true');
  toggle.focus();await act(async()=>toggle.click());
  expect(document.activeElement).toBe(toggle);
  expect(toggle.getAttribute('aria-label')).toBe('Expand sidebar');
  expect(host.querySelector('.sidebar.is-collapsed')).not.toBeNull();
  expect(host.querySelector('nav button')?.getAttribute('aria-label')).toBe('My PRs');
  expect(dom.localStorage.getItem(key)).toBe('true');
  await act(async()=>root.unmount());root=createRoot(host);await render();
  expect(host.querySelector('.sidebar.is-collapsed')).not.toBeNull();
  await act(async()=>host.querySelector<HTMLButtonElement>('.sidebar-toggle')!.click());
  expect(dom.localStorage.getItem(key)).toBe('false');
  expect(host.querySelector('.sidebar.is-collapsed')).toBeNull();
});

test('small screens start collapsed and respect an explicitly expanded preference',async()=>{
  dom.matchMedia=(()=>({matches:true})) as typeof dom.matchMedia;
  await render();expect(host.querySelector('.sidebar.is-collapsed')).not.toBeNull();
  await act(async()=>host.querySelector<HTMLButtonElement>('.sidebar-toggle')!.click());
  await act(async()=>root.unmount());root=createRoot(host);await render();
  expect(host.querySelector('.sidebar.is-collapsed')).toBeNull();
});
