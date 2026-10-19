import React,{useEffect,useState,type ReactNode} from 'react';
import {ArrowLeft,ArrowRight,PanelLeftClose,PanelLeftOpen,Plus} from 'lucide-react';

const preferenceKey='gitjuke.sidebar-collapsed.v1';

export function useSidebarState(){
  const [collapsed,setCollapsed]=useState(()=>{
    try{
      const saved=window.localStorage.getItem(preferenceKey);
      if(saved!==null)return saved==='true';
    }catch{/* The sidebar still works when browser storage is unavailable. */}
    return window.matchMedia('(max-width:640px)').matches;
  });
  useEffect(()=>{
    try{window.localStorage.setItem(preferenceKey,String(collapsed));}catch{/* Keep the in-memory preference. */}
  },[collapsed]);
  return {collapsed,setCollapsed};
}

export function Sidebar({children,onAddPullRequest,collapsed,setCollapsed,history}:{children:ReactNode;onAddPullRequest?:()=>void;collapsed:boolean;setCollapsed:React.Dispatch<React.SetStateAction<boolean>>;history?:{canBack:boolean;canForward:boolean;back:()=>void;forward:()=>void}}){
  const label=collapsed?'Expand sidebar':'Collapse sidebar';
  const nativeMac=window.navigator.platform.startsWith('Mac')&&'__TAURI_INTERNALS__' in window;
  const mac=window.navigator.platform.startsWith('Mac');
  return <>
    <header className={`window-toolbar${nativeMac?' native-mac':''}`} data-tauri-drag-region aria-label="Window toolbar">
      <div className="history-controls" role="group" aria-label="Navigation history">
        <button className="icon-button" type="button" title={`Go back (${mac?'⌘[ or ':''}Alt+←)`} aria-label="Go back" aria-keyshortcuts={mac?'Alt+ArrowLeft Meta+[':'Alt+ArrowLeft'} disabled={!history?.canBack} onClick={history?.back}><ArrowLeft size={18}/></button>
        <button className="icon-button" type="button" title={`Go forward (${mac?'⌘] or ':''}Alt+→)`} aria-label="Go forward" aria-keyshortcuts={mac?'Alt+ArrowRight Meta+]':'Alt+ArrowRight'} disabled={!history?.canForward} onClick={history?.forward}><ArrowRight size={18}/></button>
      </div>
      <button className="sidebar-toggle" type="button" title={label} aria-label={label} aria-expanded={!collapsed} aria-controls="sidebar-navigation" onClick={()=>setCollapsed(value=>!value)}>
        {collapsed?<PanelLeftOpen size={18}/>:<PanelLeftClose size={18}/>}
      </button>
      <span className="window-title" data-tauri-drag-region>GitJuke</span>
      <button className="icon-button" type="button" title="Add pull request" aria-label="Add pull request" aria-haspopup="dialog" onClick={onAddPullRequest}><Plus size={19}/></button>
    </header>
    <aside className={`sidebar${collapsed?' is-collapsed':''}`} aria-label="Sidebar">
      <div className="sidebar-header">
        <div className="brand"><span className="brand-icon" aria-hidden="true"/><span>gitjuke<span className="brand-dot">.</span></span></div>
      </div>
      {children}
    </aside>
  </>;
}
