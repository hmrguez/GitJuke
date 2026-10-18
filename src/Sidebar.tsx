import React,{useEffect,useState,type ReactNode} from 'react';
import {PanelLeftClose,PanelLeftOpen} from 'lucide-react';

const preferenceKey='gitjuke.sidebar-collapsed.v1';

export function Sidebar({children}:{children:ReactNode}){
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
  const label=collapsed?'Expand sidebar':'Collapse sidebar';
  return <aside className={`sidebar${collapsed?' is-collapsed':''}`} aria-label="Sidebar">
    <div className="sidebar-header">
      <div className="brand"><span className="brand-icon" aria-hidden="true"/><span>gitjuke<span className="brand-dot">.</span></span></div>
      <button className="sidebar-toggle" type="button" title={label} aria-label={label} aria-expanded={!collapsed} aria-controls="sidebar-navigation" onClick={()=>setCollapsed(value=>!value)}>
        {collapsed?<PanelLeftOpen size={18}/>:<PanelLeftClose size={18}/>}
      </button>
    </div>
    {children}
  </aside>;
}
