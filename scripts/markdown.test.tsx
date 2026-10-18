import React,{act} from 'react';
import {test,expect} from 'bun:test';
import {renderToStaticMarkup} from 'react-dom/server';
import {createRoot} from 'react-dom/client';
import {Window} from 'happy-dom';
import {Markdown,markdownUrl} from '../src/Markdown';
const render=(body:string)=>(renderToStaticMarkup(<Markdown baseUrl="https://github.com/a/b/pull/1">{body}</Markdown>));
test('GitHub Markdown renders headings, nested lists, tasks, tables, quotes, code, autolinks and footnotes',()=>{
 const html=render('# Heading\n\n**Strong** and *emphasis* and ~~removed~~\n\n- [x] Done\n- [ ] Pending\n  - Nested\n\n| A | B |\n| - | - |\n| 1 | 2 |\n\n> Quoted\n\n```ts\nconst x = "<div>";\n```\n\nhttps://example.com\n\nNote[^1]\n\n[^1]: Footnote');
 for(const part of ['<h1>Heading</h1>','<strong>Strong</strong>','<em>emphasis</em>','<del>removed</del>','type="checkbox"','checked=""','<table>','<blockquote>','language-ts','&lt;div&gt;','href="https://example.com/"','Footnote'])expect(html).toContain(part);
 const targets=[...html.matchAll(/href="#([^"]+)"/g)].map(m=>m[1]);for(const id of targets)expect(html).toContain(`id="${id}"`);
});
test('safe HTML details render while scripts, event handlers, styles and unsafe URLs are removed',()=>{
 const html=render('<details><summary>More</summary>\n\n**Details**\n\n</details>\n\n<script>alert(1)</script><iframe src="https://evil.test"></iframe><img src="x" onerror="alert(1)"><a href="javascript:alert(1)" style="color:red" onclick="alert(1)">bad</a>');
 expect(html).toContain('<details>');expect(html).toContain('<summary>More</summary>');expect(html).toContain('<strong>Details</strong>');expect(html).not.toMatch(/<(?:script|iframe)\b|onerror=|onclick=|style=|javascript:/);
});
test('resource-heavy formatting falls back to complete source without truncation',()=>{
 const body='**bold**\n\n'.repeat(5000)+'final text';const html=render(body);
 expect(html).toContain('Shown as plain text');expect(html).toContain('final text');expect(html).not.toContain('<strong>');
});
test('links resolve against GitHub and reject local files, executable schemes and credentials',()=>{
 expect(markdownUrl('../2','https://github.com/a/b/pull/1')).toBe('https://github.com/a/b/2');
 for(const url of ['javascript:alert(1)','file:///etc/passwd','data:text/html,test','https://user:pass@example.com','tauri://localhost'])expect(markdownUrl(url)).toBeUndefined();
});
test('images load lazily without a click, can retry failures and external links invoke the native opener',async()=>{
 const dom=new Window();Object.assign(globalThis,{window:dom,document:dom.document,IS_REACT_ACT_ENVIRONMENT:true,isTauri:true});
 const calls:{name:string;args:unknown}[]=[];(window as any).isTauri=true;(window as any).__TAURI_INTERNALS__={invoke:async(name:string,args:unknown)=>{calls.push({name,args});}};
 const host=document.createElement('div');document.body.append(host);const root=createRoot(host);
 await act(async()=>root.render(<Markdown baseUrl="https://github.com/a/b/pull/1">{'![Screenshot](https://example.com/image.png)\n\n[Docs](https://example.com/docs)'}</Markdown>));
 expect(host.querySelector('img')?.getAttribute('src')).toBe('https://example.com/image.png');
 expect(host.querySelector('img')?.getAttribute('loading')).toBe('lazy');
 expect(host.querySelector('button')).toBeNull();expect(calls).toHaveLength(0);
 await act(async()=>host.querySelector('img')!.dispatchEvent(new window.Event('error')));
 expect(host.querySelector('img')).toBeNull();expect(host.querySelector('button')?.textContent).toBe('Retry image: Screenshot');
 await act(async()=>host.querySelector('button')!.click());expect(host.querySelector('img')?.referrerPolicy).toBe('no-referrer');
 await act(async()=>host.querySelector('a')!.click());expect(calls).toEqual([{name:'open_markdown_link',args:{url:'https://example.com/docs'}}]);
 await act(async()=>root.unmount());host.remove();
});
