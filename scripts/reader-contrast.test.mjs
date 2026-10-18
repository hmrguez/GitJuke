import {test} from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {presets} from '../src/api.ts';

const theme=readFileSync(new URL('../src/theme.css',import.meta.url),'utf8');
const discussion=readFileSync(new URL('../src/conversation.css',import.meta.url),'utf8');
const light=theme.match(/:root\[data-mode=light\]\{([^}]+)\}/)[1];
const rgb=hex=>hex.slice(1).match(/../g).map(v=>parseInt(v,16)/255);
const mix=(a,b,p)=>a.map((v,i)=>v*p+b[i]*(1-p));
const luminance=c=>c.map(v=>v<=.04045?v/12.92:((v+.055)/1.055)**2.4).reduce((s,v,i)=>s+v*[.2126,.7152,.0722][i],0);
const contrast=(a,b)=>{const [lo,hi]=[luminance(a),luminance(b)].sort((a,b)=>a-b);return (hi+.05)/(lo+.05);};
const weight=(css,name)=>Number(css.match(new RegExp(`--${name}:color-mix\\(in srgb,var\\(--theme-ink\\) ([\\d.]+)%`))?.[1])/100;

test('metadata remains at least 4.5:1 across approved reader surfaces in both palettes',()=>{
 const thread=discussion.match(/\.thread-entry span,\.thread-entry time\{([^}]+)\}/)[1];
 const opacity=Number(thread.match(/opacity:([\d.]+)/)?.[1]??1);
 for(const palette of Object.values(presets)){
  const ink=rgb(palette.ink),surface=rgb(palette.surface),canvas=rgb(palette.canvas);
  const p=palette.mode==='light'&&Number.isFinite(weight(light,'muted-text'))?weight(light,'muted-text'):weight(theme,'muted-text');
  const muted=mix(ink,surface,p);
  for(const [name,bg] of [['canvas',canvas],['surface',surface],['quiet',mix(ink,surface,.04)],['hover',mix(ink,surface,.07)]])assert.ok(contrast(muted,bg)>=4.5,`${palette.mode} ${name} metadata contrast ${contrast(muted,bg).toFixed(2)}`);
  const threadBase=thread.includes('color:var(--muted-text)')?muted:mix(ink,surface,weight(theme,'secondary-text'));
  const background=mix(ink,surface,.04),foreground=mix(threadBase,background,opacity);
  assert.ok(contrast(foreground,background)>=4.5,`${palette.mode} thread metadata contrast ${contrast(foreground,background).toFixed(2)}`);
 }
});

test('old/new line guides retain 4.5:1 on normal, added and deleted diff backgrounds',()=>{
 const changes=readFileSync(new URL('../src/changes.css',import.meta.url),'utf8');
 const tone=changes.match(/\.diff-row \.line-number\{[^}]*color:var\(--([\w-]+)\)/)[1];
 const added=Number(changes.match(/\.diff-row\.added\{background:color-mix\(in srgb,var\(--theme-accent\) ([\d.]+)%/)[1])/100;
 const deleted=Number(changes.match(/\.diff-row\.deleted\{background:color-mix\(in srgb,var\(--danger\) ([\d.]+)%/)[1])/100;
 for(const palette of Object.values(presets)){
  const ink=rgb(palette.ink),surface=rgb(palette.surface),canvas=rgb(palette.canvas);
  const rules=palette.mode==='light'?light:theme;
  const w=Number.isFinite(weight(rules,tone))?weight(rules,tone):weight(theme,tone);
  const fg=mix(ink,surface,w),danger=rgb(rules.match(/--danger:(#[\da-f]+)/i)[1]);
  for(const [kind,bg] of [['normal',canvas],['added',mix(rgb(palette.accent),canvas,added)],['deleted',mix(danger,canvas,deleted)]])assert.ok(contrast(fg,bg)>=4.5,`${palette.mode} ${kind} line guide contrast ${contrast(fg,bg).toFixed(2)}`);
 }
});
