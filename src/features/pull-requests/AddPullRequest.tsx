import React,{useEffect,useRef,useState} from 'react';
import {X} from 'lucide-react';

export function AddPullRequest({open,close}:{open:(repo:string,number:number)=>Promise<boolean>;close:()=>void}){
  const dialog=useRef<HTMLDialogElement>(null);
  const submitting=useRef(false);
  const [busy,setBusy]=useState(false),[error,setError]=useState('');
  useEffect(()=>{dialog.current?.showModal();dialog.current?.querySelector<HTMLInputElement>('input[name=repo]')?.focus();},[]);
  async function submit(event:React.FormEvent<HTMLFormElement>){
    event.preventDefault();
    if(submitting.current)return;
    const data=new FormData(event.currentTarget);
    submitting.current=true;setBusy(true);setError('');
    try{
      if(await open(String(data.get('repo')).trim(),Number(data.get('number'))))dialog.current?.close();
      else setError('Couldn’t open this pull request. Check the repository, PR number, and your GitHub connection.');
    }catch{setError('Couldn’t open this pull request. Try again.');}
    finally{submitting.current=false;setBusy(false);}
  }
  return <dialog ref={dialog} className="dialog add-pr-dialog" aria-labelledby="add-pr-title" onClose={close}>
    <div className="dialog-header"><h2 id="add-pr-title">Add pull request</h2><button type="button" aria-label="Close add pull request" onClick={()=>dialog.current?.close()}><X size={17}/></button></div>
    <form onSubmit={submit} aria-busy={busy}>
      <label>Repository<input name="repo" placeholder="owner/repository" pattern={String.raw`[^\s\/]+\/[^\s\/]+`} title="Enter a repository as owner/repository" required autoFocus autoCapitalize="none" spellCheck={false}/></label>
      <label>Pull request number<input name="number" type="number" min="1" step="1" required placeholder="123"/></label>
      {error&&<p className="state-banner" role="alert">{error}</p>}
      <div className="dialog-actions"><button type="button" onClick={()=>dialog.current?.close()}>Cancel</button><button type="submit" disabled={busy}>{busy?'Adding…':'Add pull request'}</button></div>
    </form>
  </dialog>;
}
