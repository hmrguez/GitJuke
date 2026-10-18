import './login.css';
import {useEffect,useRef,useState} from 'react';
import {command} from './api';
export type LoginStatus={desired:boolean;installed:boolean;registered:boolean;registrationValid:boolean;osDisabled:boolean|null;state:string;message:string};
export function LoginSettings(){
  const [status,setStatus]=useState<LoginStatus|null>(null),[busy,setBusy]=useState(false),[error,setError]=useState('');
  const sequence=useRef(0),mounted=useRef(false),changing=useRef(false);
  async function read(){
    if(changing.current)return;const ticket=++sequence.current;
    try{const next=await command<LoginStatus>('login_status');if(mounted.current&&ticket===sequence.current){setStatus(next);}}
    catch(e){if(mounted.current&&ticket===sequence.current)setError(String(e));}
  }
  useEffect(()=>{mounted.current=true;void read();const focus=()=>void read();window.addEventListener('focus',focus);const timer=setInterval(focus,15000);return()=>{mounted.current=false;++sequence.current;clearInterval(timer);window.removeEventListener('focus',focus);};},[]);
  async function change(enabled:boolean,repair=false){
    if(changing.current)return;changing.current=true;setBusy(true);setError('');++sequence.current;
    try{const next=await command<LoginStatus>('set_launch_at_login',{enabled,repair});if(mounted.current)setStatus(next);}
    catch(e){if(mounted.current)setError(String(e));}
    finally{changing.current=false;if(mounted.current){setBusy(false);void read();}}
  }
  return <div className="login-preferences" aria-label="Launch at login" aria-busy={busy}>
    <h3>Launch at login</h3>
    <label className="login-toggle"><input type="checkbox" checked={status?.desired??false} disabled={busy||!status||(!status.installed&&!status.desired)} onChange={e=>void change(e.target.checked)}/> Launch GitJuke at login</label>
    <p role="status" className="dialog-note">{!status?'Checking…':!status.installed?'Move GitJuke to Applications to use this setting.':status.osDisabled?'macOS has turned this off. Open Login Items in System Settings or try fixing it below.':status.desired&&(!status.registered||!status.registrationValid)?'Launch at login needs fixing.':status.state==='unexpectedRegistration'?'macOS may still start GitJuke. Turn off launch at login again below.':status.desired&&status.osDisabled===null?'Check Login Items in System Settings to confirm GitJuke can start.':status.desired?'GitJuke will start when you sign in.':'Launch at login is off.'}</p>

    <div className="login-actions">{status?.desired&&<button disabled={busy||!status.installed} onClick={()=>void change(true,true)}>Fix launch at login</button>}
    {status?.registered&&<button disabled={busy} onClick={()=>void change(false)}>Disable launch at login</button>}
    </div>
    {error&&<p role="alert" className="state-banner">Couldn’t change launch at login. Try again.</p>}
  </div>;
}
