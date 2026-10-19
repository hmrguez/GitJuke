import {Skeleton} from '../../components/Skeleton';
import React,{useEffect,useState} from 'react';
import {command,type BuildInfo} from '../../lib/ipc';

export function BuildInformation(){
 const [info,setInfo]=useState<BuildInfo|null>(null),[failed,setFailed]=useState(false),[retry,setRetry]=useState(0);
 useEffect(()=>{
  let active=true;setFailed(false);
  void command<BuildInfo>('build_info').then(value=>{if(active)setInfo(value);}).catch(()=>{if(active)setFailed(true);});
  return()=>{active=false;};
 },[retry]);
 return <div className="build-information">
  {info?<><p>GitJuke {info.version}</p><dl>
   <dt>Build</dt><dd>{info.buildId}</dd>
   <dt>Source</dt><dd>{info.sourceState==='unavailable'?'Unavailable':<>{info.revision}{info.sourceState==='modified'?' · modified sources':''}</>}</dd>
   <dt>Target</dt><dd>{info.target} · {info.profile}</dd>
   <dt>Data format</dt><dd>{info.schemaVersion}</dd>
  </dl></>:failed?<p role="status">Build information couldn’t be loaded. <button onClick={()=>setRetry(n=>n+1)}>Try loading build information again</button></p>:<Skeleton shape="build" label="Loading build information"/>}
 </div>;
}
