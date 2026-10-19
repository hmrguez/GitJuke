import React from 'react';
import './skeleton.css';

type Shape='pull-requests'|'activity'|'detail'|'conversation'|'text'|'files'|'diff'|'checks'|'log'|'build';
type Props={shape:Shape;label:string;count?:number};
const widths=[92,76,85,58,81,67,90,48];
function Bar({width=100,className=''}:{width?:number;className?:string}){
 return <span className={`skeleton-bar ${className}`} style={{width:`${width}%`}}/>;
}
function TextLines(){return <div className="skeleton-text"><Bar width={94}/><Bar width={87}/><Bar width={62}/></div>;}

/** One announcement per loading region; decorative placeholders never enter the focus order. */
export function Skeleton({shape,label,count}:Props){
 const rows=count??(shape==='pull-requests'||shape==='activity'?4:shape==='checks'||shape==='files'?3:shape==='conversation'?2:1);
 return <div className={`skeleton skeleton-${shape}`} role="status" aria-label={label}>
  <span className="skeleton-status">{label}</span>
  <div aria-hidden="true" className="skeleton-content">{Array.from({length:rows},(_,i)=>{
   switch(shape){
    case 'pull-requests':case 'activity':return <div className="stack-node skeleton-node" key={i}><Bar width={57}/><div className="skeleton-row">{shape==='pull-requests'&&<span className="skeleton-bar skeleton-icon"/>}<Bar width={widths[i%widths.length]}/></div><Bar width={44}/></div>;
    case 'conversation':return <div className="conversation-message skeleton-message" key={i}><header><Bar width={24}/><Bar width={18}/></header><TextLines/></div>;
    case 'text':return <TextLines key={i}/>;
    case 'detail':return <React.Fragment key={i}><div className="pr-heading"><Bar width={16} className="skeleton-badge"/><Bar width={78} className="skeleton-title"/><Bar width={52} className="skeleton-title"/><Bar width={23}/></div><div className="tabs skeleton-tabs">{[22,20,28,18].map((w,n)=><Bar key={n} width={w}/>)}</div><div className="overview"><TextLines/><TextLines/></div></React.Fragment>;
    case 'files':return <div className="skeleton-file skeleton-row" key={i}><Bar width={64-i*9}/><Bar width={17}/></div>;
    case 'diff':case 'log':return <div className={`skeleton-code ${shape==='diff'?'diff-viewport':'description check-log-text'}`} key={i}>{widths.map((w,n)=><div className="skeleton-code-line" key={n}><span className="skeleton-gutter"><Bar/></span>{shape==='diff'&&<span className="skeleton-gutter"><Bar/></span>}<span className="skeleton-code-source"><Bar width={w}/></span></div>)}</div>;
    case 'checks':return <div className="check-row skeleton-check" key={i}><Bar width={widths[i%widths.length]}/><Bar width={100} className="skeleton-badge"/><Bar width={24}/></div>;
    case 'build':return <React.Fragment key={i}><Bar width={30}/><div className="skeleton-build-grid">{[56,90,44,30].map((w,n)=><React.Fragment key={n}><Bar width={80}/><Bar width={w}/></React.Fragment>)}</div></React.Fragment>;
   }
  })}</div>
 </div>;
}
