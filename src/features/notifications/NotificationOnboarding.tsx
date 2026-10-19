import React,{useEffect,useRef} from 'react';
import type {AlertSettings} from '../../lib/ipc';
import {NotificationControls} from './NotificationControls';

export function NotificationOnboarding({settings,action}:{settings:AlertSettings;action:(name:string,args?:Record<string,unknown>)=>Promise<void>}){
 const dialog=useRef<HTMLDialogElement>(null);useEffect(()=>{dialog.current?.showModal();},[]);
 return <dialog ref={dialog} className="dialog" aria-label="Enable notifications" onCancel={e=>{e.preventDefault();void action('set_alert_choices',{choices:{enabled:false,previews:false,categories:[true,true,true,true,true]}});}}><NotificationControls settings={settings} action={action} onboarding/></dialog>;
}
