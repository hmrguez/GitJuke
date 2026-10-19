import React, {useEffect, useRef, useState} from 'react';
import {command} from '../lib/ipc';
import {BuildInformation} from '../features/preferences/BuildInformation';
export type StartupReport={ready:boolean;databasePath:string;message:string;databaseState:'missing'|'legacy'|'current'|'newer'|'corrupt'|'unreadable'};
export function Startup({children}:{children:React.ReactNode}){
  const [report,setReport]=useState<StartupReport|null>(null);
  const [error,setError]=useState(''),[busy,setBusy]=useState(false);
  const serial=useRef(0);
  useEffect(()=>{const request=++serial.current;void command<StartupReport>('startup_status').then(r=>{if(request===serial.current)setReport(r);}).catch(e=>{if(request===serial.current)setError(String(e));});return()=>{++serial.current;};},[]);
  async function retry(){const request=++serial.current;setBusy(true);setError('');try{const next=await command<StartupReport>('retry_startup');if(request===serial.current)setReport(next);}catch(e){if(request===serial.current)setError(String(e));}finally{if(request===serial.current)setBusy(false);}}
  if(report?.ready)return <>{children}</>;
  return <main className="startup-recovery" aria-labelledby="startup-heading"><div className="startup-titlebar" data-tauri-drag-region/><h1 id="startup-heading">{report?'Local storage needs attention':'Opening GitJuke'}</h1>
    <p role="status">{report?'GitJuke couldn’t open your saved data.':'Checking your saved data…'}</p>
    {report&&<><p>{({missing:'Your saved data couldn’t be found.',legacy:'Your saved data needs an update before GitJuke can open it.',current:'Your saved data couldn’t be opened. Try again after checking storage.',newer:'Update GitJuke to open data saved by a newer version.',corrupt:'Your saved data appears damaged. Restore a working backup before trying again.',unreadable:'GitJuke can’t access the folder containing your saved data.'})[report.databaseState]}</p><p>Check that your Mac has enough free space and that you can open and save files in the folder below.</p><details><summary>Data folder</summary><pre>{report.databasePath}</pre><p>Quit GitJuke before restoring a backup, and keep all the files in this folder together.</p></details><p>Trying again keeps your account, preferences and read history.</p></>}
    {error&&<p role="alert">Couldn’t open your saved data. Check the folder and try again.</p>}
    <button disabled={busy} onClick={()=>void retry()}>{busy?'Checking…':'Try again'}</button>
    <p>You can quit GitJuke with ⌘Q. Your saved data won’t be reset.</p>
    {report&&<details><summary>About this build</summary><BuildInformation/></details>}
  </main>;
}
