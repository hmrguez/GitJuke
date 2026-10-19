import {useEffect,useState} from 'react';
import {nativeMac} from './native-menu';

// Session-only text enlargement. Never changes source limits, identity or seen authorization.
export const textSizes=[100,125,150,175,200] as const;
export function useReaderTextSize(){
 const [size,setSize]=useState(100);
 useEffect(()=>{
  document.documentElement.style.setProperty('--text-scale',String(size/100));
  document.documentElement.dataset.textZoom=String(size);
 },[size]);
 useEffect(()=>{
  const key=(e:KeyboardEvent)=>{
   if(e.defaultPrevented)return;
   // Native menu accelerators own these keys on macOS; avoid applying a step twice.
   if(nativeMac()&&e.metaKey&&e.key!=='=')return;
   if(!(e.metaKey||e.ctrlKey)||e.altKey||e.isComposing)return;
   if(!['+','=','-','0'].includes(e.key))return;
   e.preventDefault();
   setSize(n=>e.key==='0'?100:Math.max(100,Math.min(200,n+(e.key==='-'?-25:25))));
  };
  window.addEventListener('keydown',key);return()=>window.removeEventListener('keydown',key);
 },[]);
 return {size,setSize};
}
export function compactReader(){return window.matchMedia('(max-width: 960px)').matches||Number(document.documentElement.dataset.textZoom??100)>100;}

export function readerListKey(e:KeyboardEvent){
 if(e.metaKey||e.ctrlKey||e.altKey||e.isComposing||document.querySelector('dialog[open]'))return;
 const el=e.target as HTMLElement|null;
 if(el?.closest('input,textarea,select,[contenteditable="true"]'))return;
 if(e.key==='/'){
  const search=document.querySelector<HTMLInputElement>('.search-field input');
  if(search&&search.getClientRects().length){e.preventDefault();search.focus();}
 }else if(e.key==='j'||e.key==='k'){
  const items=[...document.querySelectorAll<HTMLButtonElement>('.stack-navigator .stack-node')].filter(el=>el.getClientRects().length);
  if(!items.length)return;
  const index=items.indexOf(document.activeElement as HTMLButtonElement);
  const next=index<0?(e.key==='j'?0:items.length-1):Math.max(0,Math.min(items.length-1,index+(e.key==='j'?1:-1)));
  e.preventDefault();items[next].focus();
 }
}
