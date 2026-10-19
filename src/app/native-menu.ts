import {useEffect,useRef,useState} from 'react';
import {listen} from '@tauri-apps/api/event';
import {command} from '../lib/ipc';

export const nativeMac=()=>window.navigator.platform.startsWith('Mac')&&'__TAURI_INTERNALS__' in window;
export type ReaderMenuState={modal:boolean;selected:boolean;filter:boolean;collapsed:boolean;textSize:number;paused:boolean;canMonitor:boolean};

// Register once, but always dispatch against the latest committed reader state.
export function useNativeMenu(state:Omit<ReaderMenuState,'modal'>,dispatch:(id:string)=>void){
  const handler=useRef(dispatch);
  useEffect(()=>{handler.current=dispatch;});
  const [ready,setReady]=useState(false),[modal,setModal]=useState(false);
  useEffect(()=>{
    if(!nativeMac())return;
    let disposed=false,unlisten:(()=>void)|undefined;
    const observer=new window.MutationObserver(()=>setModal(!!document.querySelector('dialog[open]')));
    observer.observe(document.body,{subtree:true,childList:true,attributes:true,attributeFilter:['open']});
    setModal(!!document.querySelector('dialog[open]'));
    void listen<string>('reader-menu',event=>handler.current(event.payload)).then(stop=>{
      if(disposed)stop();else{unlisten=stop;setReady(true);}
    }).catch(()=>{});
    return()=>{disposed=true;observer.disconnect();unlisten?.();};
  },[]);
  const {selected,filter,collapsed,textSize,paused,canMonitor}=state;
  useEffect(()=>{
    if(!ready)return;
    void command('sync_reader_menu',{state:{modal,selected,filter,collapsed,textSize,paused,canMonitor}}).catch(()=>{});
  },[ready,modal,selected,filter,collapsed,textSize,paused,canMonitor]);
}
