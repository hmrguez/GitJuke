import {useEffect,useId,useLayoutEffect,useRef,useState,type CSSProperties,type KeyboardEvent} from 'react';
import {Check,ChevronDown} from 'lucide-react';
import './dropdown.css';

export type DropdownOption<T extends string|number>={value:T;label:string};

/** A controlled, select-only combobox. Focus stays on the trigger while browsing. */
export function Dropdown<T extends string|number>({label,value,options,onChange,disabled=false}:{
  label:string;value:T;options:readonly DropdownOption<T>[];onChange:(value:T)=>void;disabled?:boolean;
}){
  const id=useId(),root=useRef<HTMLDivElement>(null),trigger=useRef<HTMLButtonElement>(null);
  const [open,setOpen]=useState(false),[active,setActive]=useState(0);
  const [position,setPosition]=useState<CSSProperties>({});
  const search=useRef({text:'',time:0});
  const selected=options.findIndex(option=>option.value===value);
  const expanded=open&&!disabled&&options.length>0;
  function show(index=Math.max(0,selected)){setActive(index);setOpen(true);search.current.text='';}
  function choose(index:number){const option=options[index];if(option)onChange(option.value);setOpen(false);trigger.current?.focus();}

  useLayoutEffect(()=>{
    if(!expanded)return;
    function place(){
      const rect=trigger.current!.getBoundingClientRect();
      const below=window.innerHeight-rect.bottom-12,above=rect.top-12;
      const upward=below<Math.min(options.length*44+12,280)&&above>below;
      setPosition({left:Math.max(8,Math.min(rect.left,window.innerWidth-rect.width-8)),width:rect.width,
        maxHeight:Math.max(0,upward?above:below),...(upward?{bottom:window.innerHeight-rect.top+6}:{top:rect.bottom+6})});
    }
    place();window.addEventListener('resize',place);window.addEventListener('scroll',place,true);
    return()=>{window.removeEventListener('resize',place);window.removeEventListener('scroll',place,true);};
  },[expanded,options.length]);
  useEffect(()=>{
    if(expanded)document.getElementById(`${id}-${active}`)?.scrollIntoView({block:'nearest'});
  },[expanded,active,id]);
  useEffect(()=>{
    if(!expanded)return;
    const outside=(event:PointerEvent)=>{if(!root.current?.contains(event.target as Node))setOpen(false);};
    document.addEventListener('pointerdown',outside);
    return()=>document.removeEventListener('pointerdown',outside);
  },[expanded]);

  function keyDown(event:KeyboardEvent<HTMLButtonElement>){
    if(event.altKey||event.ctrlKey||event.metaKey||event.nativeEvent.isComposing)return;
    const last=options.length-1;if(last<0)return;
    if(event.key==='Escape'&&expanded){event.preventDefault();event.stopPropagation();setOpen(false);return;}
    if(event.key==='Tab'){setOpen(false);return;}
    if(['ArrowDown','ArrowUp','Home','End'].includes(event.key)){
      event.preventDefault();
      const next=event.key==='Home'?0:event.key==='End'?last:expanded?Math.max(0,Math.min(last,active+(event.key==='ArrowDown'?1:-1))):Math.max(0,selected);
      if(expanded)setActive(next);else show(next);
    }else if(event.key==='Enter'||event.key===' '){
      event.preventDefault();if(expanded)choose(active);else show();
    }else if(event.key.length===1){
      const now=Date.now(),previous=now-search.current.time<700?search.current.text:'';
      const text=(previous+event.key).toLocaleLowerCase();search.current={text,time:now};
      const match=options.findIndex(option=>option.label.toLocaleLowerCase().startsWith(text));
      if(match>=0){event.preventDefault();setActive(match);setOpen(true);}
    }
  }

  return <div className="dropdown" ref={root} onBlur={event=>{if(!event.currentTarget.contains(event.relatedTarget))setOpen(false);}}>
    <label className="dropdown-label" id={`${id}-label`} htmlFor={`${id}-trigger`}>{label}</label>
    <button ref={trigger} id={`${id}-trigger`} type="button" className="dropdown-trigger" role="combobox"
      aria-labelledby={`${id}-label`} aria-haspopup="listbox" aria-expanded={expanded} aria-controls={expanded?`${id}-list`:undefined}
      aria-activedescendant={expanded?`${id}-${active}`:undefined} disabled={disabled||!options.length}
      onClick={()=>expanded?setOpen(false):show()} onKeyDown={keyDown}>
      <span>{options[selected]?.label??'Select…'}</span><ChevronDown size={14} aria-hidden="true"/>
    </button>
    {expanded&&<div className="dropdown-menu" id={`${id}-list`} role="listbox" aria-labelledby={`${id}-label`} style={position}>
      {options.map((option,index)=><div key={option.value} id={`${id}-${index}`} role="option" aria-selected={option.value===value}
        className={`dropdown-option${index===active?' is-active':''}`} onPointerMove={()=>setActive(index)}
        onMouseDown={event=>event.preventDefault()} onClick={()=>choose(index)}>
        <span>{option.label}</span>{option.value===value&&<Check size={14} aria-hidden="true"/>}
      </div>)}
    </div>}
  </div>;
}
