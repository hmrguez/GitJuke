import React,{memo,useId,useMemo,useState} from 'react';
import ReactMarkdown,{type Options} from 'react-markdown';
import remarkGfm from 'remark-gfm';
import rehypeRaw from 'rehype-raw';
import rehypeSanitize,{defaultSchema} from 'rehype-sanitize';
import {command} from './api';
import './markdown.css';

// Relative GitHub links resolve against the source record, never the local webview.
export function markdownUrl(value:string,baseUrl?:string):string|undefined{
 try{const url=new URL(value,baseUrl);return ['https:','http:'].includes(url.protocol)&&!url.username&&!url.password?url.href:undefined;}catch{return undefined;}
}
function Image({src,alt,title}:{src?:string;alt?:string;title?:string}){
 const [failed,setFailed]=useState(false);
 if(!src)return <span className="markdown-image-note">{alt||'Image unavailable'}</span>;
 return !failed?<img src={src} alt={alt??''} title={title} loading="lazy" referrerPolicy="no-referrer" onError={()=>setFailed(true)}/>:<button type="button" className="markdown-image-note" onClick={()=>setFailed(false)} title={src}>Retry image{alt?`: ${alt}`:''}</button>;
}
// A source can contain many thousands of tiny Markdown nodes. Keep the full text
// readable when formatting would exceed its share of the page's DOM budget.
function boundedTree(source:string,limit:number,prefix:string){
 return ()=> (tree:{children:unknown[]})=>{
  let count=0;const pending:unknown[]=[tree];
  while(pending.length){
   const node=pending.pop() as {children?:unknown[];properties?:{id?:string;name?:string;href?:string;ariaDescribedBy?:string[]}};
   if(++count>limit){tree.children=[{type:'element',tagName:'p',properties:{className:['markdown-limit']},children:[{type:'text',value:'Shown as plain text.'}]},{type:'element',tagName:'pre',properties:{className:['markdown-source']},children:[{type:'text',value:source}]}];return;}
   const props=node.properties;
   if(props?.name)props.name=`${prefix}-${props.name}`;
   if(props?.ariaDescribedBy)props.ariaDescribedBy=props.ariaDescribedBy.map(value=>`${prefix}-${value}`);
   if(props?.id)props.id=`${prefix}-${props.id}`;
   if(props?.href?.startsWith('#'))props.href=`#${prefix}-${props.href.slice(1)}`;
   if(node.children)pending.push(...node.children);
  }
 };
}
export const Markdown=memo(function Markdown({children,baseUrl,nodeLimit=4000}:{children:string;baseUrl?:string;nodeLimit?:number}){
 const id=useId().replaceAll(':',''),[error,setError]=useState('');
 const plugins=useMemo<Options['rehypePlugins']>(()=>[rehypeRaw,[rehypeSanitize,{...defaultSchema,clobberPrefix:''}],boundedTree(children,nodeLimit,id)],[children,nodeLimit,id]);
 return <div className="markdown-body"><ReactMarkdown remarkPlugins={[remarkGfm]} rehypePlugins={plugins}
  urlTransform={(url)=>url.startsWith('#')?url:markdownUrl(url,baseUrl)??''}
  components={{
   a:({node,href,children,...props})=>href?<a {...props} href={href} rel="noreferrer" onClick={e=>{if(href.startsWith('#'))return;e.preventDefault();void command('open_markdown_link',{url:href}).catch(e=>setError(String(e)));}}>{children}</a>:<span>{children}</span>,
   img:({src,alt,title})=><Image key={typeof src==='string'?src:''} src={typeof src==='string'?src:undefined} alt={alt} title={title}/>,
   pre:({children})=><pre tabIndex={0} aria-label="Code block">{children}</pre>,
   table:({children})=><div className="markdown-table" tabIndex={0} role="region" aria-label="Markdown table"><table>{children}</table></div>,
  }}>{children}</ReactMarkdown>{error&&<p role="alert" className="markdown-limit">Couldn’t open this link. Please try again.</p>}</div>;
});
