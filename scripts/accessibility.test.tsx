import React,{act} from 'react';
import {createRoot} from 'react-dom/client';
import {Window} from 'happy-dom';
import {expect,test} from 'bun:test';
import {useReaderTextSize} from '../src/reader-accessibility';

test('native text commands reach actual 200% font scale, clamp, reset and detach',async()=>{
 const dom=new Window();Object.assign(globalThis,{window:dom,document:dom.document,IS_REACT_ACT_ENVIRONMENT:true});
 const host=document.createElement('div');document.body.append(host);const root=createRoot(host);
 function Probe(){const {size}=useReaderTextSize();return <output>{size}</output>;}
 await act(async()=>root.render(<Probe/>));
 const key=async(value:string,extra={})=>act(async()=>{window.dispatchEvent(new dom.KeyboardEvent('keydown',{key:value,metaKey:true,cancelable:true,...extra}) as any);});
 for(let i=0;i<8;i++)await key('+');
 expect(host.textContent).toBe('200');expect(document.documentElement.style.getPropertyValue('--text-scale')).toBe('2');
 expect(document.documentElement.dataset.textZoom).toBe('200');
 await key('-');expect(host.textContent).toBe('175');await key('0');expect(host.textContent).toBe('100');
 await key('=',{altKey:true});expect(host.textContent).toBe('100');
 await act(async()=>root.unmount());await key('+');expect(document.documentElement.dataset.textZoom).toBe('100');host.remove();
});

test('list shortcuts respect editing, modifiers, modal boundaries, hidden lists and exact rows',async()=>{
 const {readerListKey}=await import('../src/reader-accessibility');
 const dom=new Window();Object.assign(globalThis,{window:dom,document:dom.document});
 document.body.innerHTML='<section class="stack-navigator"><label class="search-field"><input/></label><button class="stack-node">one</button><button class="stack-node">two</button></section><textarea></textarea>';
 const rows=[...document.querySelectorAll<HTMLButtonElement>('button')],search=document.querySelector('input')!;
 for(const el of [...rows,search])el.getClientRects=()=>[{}] as any;
 const key=(key:string,target:Element=document.body,extra={})=>readerListKey({key,target,preventDefault(){},...extra} as any);
 key('j');expect(document.activeElement).toBe(rows[0]);key('j');expect(document.activeElement).toBe(rows[1]);key('k');expect(document.activeElement).toBe(rows[0]);
 key('/',rows[0]);expect(document.activeElement).toBe(search);
 key('j',search);expect(document.activeElement).toBe(search);key('k',document.body,{metaKey:true});expect(document.activeElement).toBe(search);
 document.body.insertAdjacentHTML('beforeend','<dialog open></dialog>');key('j');expect(document.activeElement).toBe(search);document.querySelector('dialog')!.remove();
 rows.forEach(row=>row.getClientRects=()=>[] as any);key('j');expect(document.activeElement).toBe(search);
});
