export type DiffRow={text:string;old:number|null;next:number|null;kind:'hunk'|'added'|'deleted'|'context'|'note'};
export type DiffModel={rows:DiffRow[];hunks:{row:number;label:string}[]};
export function parseDiff(source:string):DiffModel{
  const lines=source.split('\n');if(lines.at(-1)==='')lines.pop();
  let old=0,next=0;
  const hunks:DiffModel['hunks']=[];
  const rows=lines.map((text,row):DiffRow=>{
    const h=text.match(/^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@/);
    if(h){old=Number(h[1]);next=Number(h[2]);hunks.push({row,label:text});return {text,old:null,next:null,kind:'hunk'};}
    if(text.startsWith('\\'))return {text,old:null,next:null,kind:'note'};
    const kind=text[0]==='+'?'added':text[0]==='-'?'deleted':'context';
    return {text,old:kind==='added'?null:old++,next:kind==='deleted'?null:next++,kind};
  });
  return {rows,hunks};
}
export function visibleRange(top:number,height:number,rowHeight:number,total:number,paged:boolean,page:number){
  const start=paged?Math.min(page*200,Math.max(0,total-1)):Math.max(0,Math.floor(top/rowHeight)-24);
  return {start,end:Math.min(total,start+(paged?200:Math.min(400,Math.ceil(height/rowHeight)+48)))};
}
