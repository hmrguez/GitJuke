import React,{useState} from 'react';
import type {AlertSettings} from './api';
import './notifications.css';
export function NotificationControls({settings,action,onboarding=false}:{settings:AlertSettings;action:(name:string,args?:Record<string,unknown>)=>Promise<void>;onboarding?:boolean}){
 const [busy,setBusy]=useState(false);
 async function apply(name:string,args?:Record<string,unknown>){if(busy)return;setBusy(true);try{await action(name,args);}finally{setBusy(false);}}
 const save=(patch:Partial<Pick<AlertSettings,'enabled'|'previews'|'categories'>>)=>apply('set_alert_choices',{choices:{enabled:settings.enabled,previews:settings.previews,categories:settings.categories,...patch}});
 return <section className="notification-controls" aria-label="Notifications"><h3>{onboarding?'Your activity, at your pace':'Notifications'}</h3>
  {onboarding&&<p className="dialog-note">Get notified about comments, reviews, mentions, review requests and completed checks.</p>}
  <p className="dialog-note">Notifications hide PR details by default. Pausing updates also pauses notifications.</p>
  <p role="status" className="dialog-note">{settings.permission==='denied'?'Notifications are blocked in System Settings.':settings.permission==='unknown'?'Checking notification settings…':settings.enabled&&settings.permission==='granted'?'Notifications are on.':'Notifications are off.'}</p>
  <div className="pagination"><button disabled={busy} onClick={()=>void(settings.enabled?save({enabled:false}):apply('enable_notifications'))}>{settings.enabled?'Disable notifications':'Enable notifications'}</button>
  {onboarding&&<button disabled={busy} onClick={()=>void save({enabled:false})}>Skip for now</button>}
  {settings.permission==='denied'&&<button disabled={busy} onClick={()=>void apply('notification_settings')}>Open System Settings</button>}</div>
  {!onboarding&&<><fieldset disabled={busy}><legend>Notify me about</legend>{['Comments','Reviews','Review requests','Mentions','Completed checks'].map((label,k)=><label key={label}><input type="checkbox" checked={settings.categories[k]} onChange={e=>{const categories=[...settings.categories];categories[k]=e.target.checked;void save({categories});}}/>{label}</label>)}</fieldset>
  <label><input type="checkbox" disabled={busy} checked={settings.previews} onChange={e=>void save({previews:e.target.checked})}/>Show repository, PR title and author in notifications</label><p className="dialog-note">This also shows details from private pull requests. Comment text and logs are never included.</p></>}
 </section>;
}
