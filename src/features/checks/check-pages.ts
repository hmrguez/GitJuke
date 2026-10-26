import {command,type CheckPage} from '../../lib/ipc';

// Drain local IPC pages too: opening Checks must never reveal unfetched results.
export async function readAllChecks(pr:{repo:string;number:number}):Promise<CheckPage>{
 const first=await command<CheckPage>('read_checks',{...pr,offset:0});
 const items=[...first.items];let offset=first.nextOffset;
 const visited=new Set<number>([0]);
 while(offset!=null){
  if(visited.has(offset))throw new Error('Repeated checks page');
  visited.add(offset);
  const page=await command<CheckPage>('read_checks',{...pr,offset});
  if(page.head!==first.head||page.revision!==first.revision)throw new Error('Checks changed during loading');
  items.push(...page.items);offset=page.nextOffset;
 }
 return {...first,items,nextOffset:null};
}
