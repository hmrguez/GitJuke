import {parseDiff} from './diff-model';
self.onmessage=async({data}:{data:{source:string;bytes:number;rows:number;digest:string}})=>{
  try{
    const bytes=new TextEncoder().encode(data.source);
    const hash=Array.from(new Uint8Array(await crypto.subtle.digest('SHA-256',bytes)),b=>b.toString(16).padStart(2,'0')).join('');
    if(bytes.length!==data.bytes||hash!==data.digest)throw new Error('Patch transfer was incomplete or obsolete. Reload this file.');
    const model=parseDiff(data.source);
    if(model.rows.length!==data.rows||model.rows.length>20000||bytes.length>2097152)throw new Error('Patch transfer exceeds its verified bounds.');
    self.postMessage({model});
  }catch(e){self.postMessage({error:String(e)});}
};
