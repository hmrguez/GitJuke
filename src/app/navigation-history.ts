import {useEffect,useLayoutEffect,useRef,useState} from 'react';
import type {ActivityRecord,SavedQuery} from '../lib/ipc';

export type NavigationEntry={
  view:'mine'|'review'|'activity'|'query';
  query:SavedQuery|null;
  selected:{repo:string;number:number}|null;
  tab:'overview'|'changes'|'conversation'|'checks';
  destination:ActivityRecord|null;
  notificationRecord:ActivityRecord|null;
  notificationUnavailable:boolean;
  search:string;offset:number;checkOffset:number;origin:string|null;
};

// Reader tabs and list controls belong to a destination, rather than adding steps.
function identity(entry:NavigationEntry){
  return JSON.stringify([entry.view,entry.view==='query'?entry.query?.id:null,entry.selected?.repo,entry.selected?.number]);
}

export function useNavigationHistory(entry:NavigationEntry,ready:boolean,restore:(entry:NavigationEntry)=>void,resolve:(entry:NavigationEntry)=>Promise<NavigationEntry|null>,report:(error:unknown)=>void){
  const history=useRef<{entries:(NavigationEntry|null)[];index:number}>({entries:[],index:-1});
  const handlers=useRef({restore,resolve,report});
  const operation=useRef(0);
  const [,refresh]=useState(0);
  const [busy,setBusy]=useState(false);
  useLayoutEffect(()=>{
    handlers.current={restore,resolve,report};
    if(!ready)return;
    const h=history.current,current=h.entries[h.index];
    if(current&&identity(current)===identity(entry)){h.entries[h.index]=entry;return;}
    if(current&&!current.selected&&entry.selected&&current.view===entry.view&&current.query?.id===entry.query?.id){
      h.entries[h.index]={...current,origin:entry.origin};
    }
    ++operation.current;setBusy(false);
    h.entries=h.entries.slice(0,h.index+1);h.entries.push(entry);h.index++;
    refresh(n=>n+1);
  });
  useEffect(()=>()=>{++operation.current;},[]);
  function find(direction:-1|1,from=history.current.index){
    const h=history.current;
    for(let i=from+direction;i>=0&&i<h.entries.length;i+=direction)if(h.entries[i])return i;
    return -1;
  }
  async function move(direction:-1|1){
    if(document.querySelector('dialog[open]'))return;
    const h=history.current;
    let index=find(direction);
    if(index===-1)return;
    const token=++operation.current;
    setBusy(true);
    try{
      while(index!==-1){
        const next=await handlers.current.resolve(h.entries[index]!);
        if(token!==operation.current)return;
        if(next){h.index=index;h.entries[index]=next;handlers.current.restore(next);return;}
        // Remove destinations only after an authoritative read says they are gone.
        h.entries[index]=null;index=find(direction,index);refresh(n=>n+1);
      }
    }catch(error){if(token===operation.current)handlers.current.report(error);}
    finally{if(token===operation.current)setBusy(false);}
  }
  const controls=useRef({move});
  useLayoutEffect(()=>{controls.current={move};});
  useEffect(()=>{
    const key=(event:KeyboardEvent)=>{
      if(event.defaultPrevented||event.isComposing||event.shiftKey||document.querySelector('dialog[open]'))return;
      const target=event.target as HTMLElement|null;
      if(target?.closest('input,textarea,select,[contenteditable]:not([contenteditable="false"]),[role="textbox"]'))return;
      const alt=event.altKey&&!event.ctrlKey&&!event.metaKey;
      const mac=window.navigator.platform.startsWith('Mac')&&event.metaKey&&!event.altKey&&!event.ctrlKey;
      const direction=alt&&event.key==='ArrowLeft'||mac&&event.key==='['?-1:alt&&event.key==='ArrowRight'||mac&&event.key===']'?1:null;
      if(direction===null)return;
      event.preventDefault();event.stopPropagation();void controls.current.move(direction);
    };
    const mouse=(event:MouseEvent)=>{
      if(event.button!==3&&event.button!==4)return;
      // Suppress WebView/browser history for the complete side-button gesture.
      event.preventDefault();
      if(event.type==='mouseup')void controls.current.move(event.button===3?-1:1);
    };
    window.addEventListener('keydown',key,true);
    for(const name of ['mousedown','mouseup','auxclick'] as const)window.addEventListener(name,mouse);
    return()=>{
      window.removeEventListener('keydown',key,true);
      for(const name of ['mousedown','mouseup','auxclick'] as const)window.removeEventListener(name,mouse);
    };
  },[]);
  return {canBack:!busy&&find(-1)!==-1,canForward:!busy&&find(1)!==-1,back:()=>void move(-1),forward:()=>void move(1)};
}
