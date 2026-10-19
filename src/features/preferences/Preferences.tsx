import React,{useEffect,useRef,useState} from 'react';
import {X} from 'lucide-react';
import type {Status} from '../../lib/ipc';
import {friendlyError} from '../../lib/ui-copy';
import {presets} from '../../app/theme';
import {textSizes} from '../../app/reader-accessibility';
import {Dropdown} from '../../components/Dropdown';
import {NotificationControls} from '../notifications/NotificationControls';
import {LoginSettings} from './LoginSettings';
import {BuildInformation} from './BuildInformation';

export function Preferences({status,close,action,textSize,error,initialSection='Appearance'}:{initialSection?:'Appearance'|'About';error?:string;textSize:{size:number;setSize:(size:number)=>void};status:Status|null;close:()=>void;action:(name:string,args?:Record<string,unknown>)=>Promise<void>}){
  const dialog=useRef<HTMLDialogElement>(null);
  useEffect(()=>{dialog.current?.showModal();},[]);
  const [section,setSection]=useState<'Appearance'|'Updates'|'GitHub'|'Storage'|'About'>(initialSection);
  const theme=presets[status?.settings.theme.mode??'dark'];
  return <dialog ref={dialog} onClose={close} aria-labelledby="preferences-title" className="dialog"><div className="dialog-header"><h2 id="preferences-title">Preferences</h2><button onClick={()=>dialog.current?.close()} aria-label="Close preferences"><X size={17}/></button></div>
    <div className="settings-tabs" role="group" aria-label="Preferences sections">{(['Appearance','Updates','GitHub','Storage','About'] as const).map(item=><button key={item} aria-pressed={section===item} onClick={()=>setSection(item)}>{item}</button>)}</div>
    {error&&<p className="state-banner" role="alert">{friendlyError(error)}</p>}
    <section aria-label={section}>
    {section==='About'&&<><BuildInformation/><p className="dialog-note">To update, quit GitJuke with ⌘Q, replace the app in Applications, then reopen it. Your account, preferences and read history stay on your Mac.</p></>}
    {section==='Appearance'&&<><div className="theme-presets">{(['dark','light'] as const).map(mode=><button key={mode} aria-pressed={theme.mode===mode} onClick={()=>void action('set_theme',{theme:presets[mode]})}><span className={`theme-preview ${mode}`} aria-hidden="true"/><span className="theme-caption">{mode==='dark'?'Graphite':'Paper'}{theme.mode===mode?' · selected':''}</span></button>)}</div><Dropdown label="Reader text size" value={textSize.size} options={textSizes.map(size=>({value:size,label:`${size}%`}))} onChange={textSize.setSize}/><p className="dialog-note">⌘ + / − changes text size; ⌘ 0 resets it.</p></>}
    {section==='Updates'&&<>
    <LoginSettings/>
    {(status?.settings.paused||status?.storageStopped)&&<p className="dialog-note">{status.settings.paused?'Updates are paused. Resume them from the sidebar.':'Updates are paused until storage is available.'}</p>}
    {status&&<NotificationControls settings={status.settings.alerts} action={action}/>}
    </>}
    {section==='GitHub'&&<><p className="dialog-note">{status?.identity?(status.locked?`Check the connection for ${status.identity.login} to see your pull requests.`:`Connected as ${status.identity.login}`):'Connect your GitHub account to see your pull requests.'}</p>
    <form key={`${status?.ghPath}:${status?.configDir}`} onSubmit={e=>{e.preventDefault();const f=new FormData(e.currentTarget);void action('configure_gh',{path:String(f.get('path')),config:String(f.get('config'))});}}>
      <details><summary>Connection settings</summary><label>GitHub command location<input name="path" defaultValue={status?.ghPath??'/opt/homebrew/bin/gh'} required/></label><label>Settings folder<input name="config" defaultValue={status?.configDir??''} required/></label></details><button type="submit">Check connection</button>
    </form>
    </>}
    {section==='Storage'&&<>
    {status&&<><p className="dialog-note">Downloaded content: {(status.storage.disposableBytes/1000000).toFixed(1)} MB</p>
      {status.storage.warning&&<p role="status" className="state-banner">Storage is running low. Clear downloaded content or free up space on your Mac.</p>}
      {status.storageStopped&&<p role="status" className="state-banner">Updates and notifications are paused until storage is available. You can still read previously downloaded pull requests.</p>}</>}
    <p className="dialog-note">Your account, preferences and read history will be kept.</p><button onClick={()=>void action('clear_downloads')}>Clear downloaded content</button>
    </>}
    </section>
  </dialog>;
}
