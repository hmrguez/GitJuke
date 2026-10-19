import React,{act,useState} from 'react';
import {createRoot,type Root} from 'react-dom/client';
import {Window} from 'happy-dom';
import {beforeEach,afterEach,expect,test} from 'bun:test';
import {Dropdown} from '../../src/components/Dropdown';

let dom:Window,host:HTMLElement,root:Root;
beforeEach(()=>{
  dom=new Window({url:'http://localhost'});
  dom.innerWidth=1024;dom.innerHeight=768;
  Object.assign(globalThis,{window:dom,document:dom.document,IS_REACT_ACT_ENVIRONMENT:true});
  host=document.createElement('div');document.body.append(host);root=createRoot(host);
});
afterEach(async()=>{await act(async()=>root.unmount());host.remove();});
async function render(){
  function Preferences(){
    const [size,setSize]=useState(100);
    return <div><Dropdown label="Reader text size" value={size} options={[100,125,150,175,200].map(value=>({value,label:`${value}%`}))} onChange={setSize}/><button>Next setting</button></div>;
  }
  await act(async()=>root.render(<Preferences/>));
  return host.querySelector<HTMLButtonElement>('[role=combobox]')!;
}
async function key(button:HTMLButtonElement,value:string){
  const event=new dom.KeyboardEvent('keydown',{key:value,bubbles:true,cancelable:true});
  await act(async()=>button.dispatchEvent(event as any));return event;
}

test('keyboard exploration commits only on confirmation and Escape preserves parent dialog',async()=>{
  const button=await render();button.focus();let escaped=false;
  const parentKey=(event:KeyboardEvent)=>{if(event.key==='Escape')escaped=true;};
  document.addEventListener('keydown',parentKey);
  await key(button,'ArrowDown');await key(button,'End');
  expect(button.textContent).toBe('100%');
  expect(document.getElementById(button.getAttribute('aria-activedescendant')!)?.textContent).toBe('200%');
  await key(button,'Enter');expect(button.textContent).toBe('200%');expect(document.activeElement).toBe(button);
  await key(button,' ');await key(button,'Home');
  const escape=await key(button,'Escape');
  expect(escape.defaultPrevented).toBe(true);expect(escaped).toBe(false);
  expect(button.getAttribute('aria-expanded')).toBe('false');expect(button.textContent).toBe('200%');
  await key(button,'Escape');expect(escaped).toBe(true);
  document.removeEventListener('keydown',parentKey);
});

test('pointer selection, typeahead, tab and outside dismissal retain controlled value',async()=>{
  const button=await render();button.focus();await act(async()=>button.click());
  const option=host.querySelectorAll<HTMLElement>('[role=option]')[2];
  await act(async()=>option.click());expect(button.textContent).toBe('150%');
  await key(button,'2');expect(document.getElementById(button.getAttribute('aria-activedescendant')!)?.textContent).toBe('200%');
  await key(button,'Tab');expect(button.getAttribute('aria-expanded')).toBe('false');expect(button.textContent).toBe('150%');
  await act(async()=>button.click());
  expect(host.querySelector('[aria-selected=true]')?.textContent).toBe('150%');
  await act(async()=>document.body.dispatchEvent(new dom.PointerEvent('pointerdown',{bubbles:true}) as any));
  expect(button.getAttribute('aria-expanded')).toBe('false');
  await act(async()=>button.click());await act(async()=>host.querySelector<HTMLButtonElement>('button:last-child')!.focus());
  expect(button.getAttribute('aria-expanded')).toBe('false');
});
