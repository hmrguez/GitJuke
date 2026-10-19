import assert from 'node:assert/strict';
import {test} from 'node:test';
import {parseDiff,visibleRange} from '../../src/features/changes/diff-model.ts';
test('whitespace and old/new coordinates survive multiple hunks and newline notes',()=>{
 const source='@@ -4,2 +4,3 @@ name\n same\t  \n-old\n+new  \n+é\t\n\\ No newline at end of file\n@@ -100 +101 @@\n tail';
 const m=parseDiff(source);
 assert.deepEqual(m.rows.map(r=>[r.old,r.next]),[[null,null],[4,4],[5,null],[null,5],[null,6],[null,null],[null,null],[100,101]]);
 assert.equal(m.rows[1].text,' same\t  ');assert.equal(m.rows[3].text,'+new  ');assert.equal(m.rows[4].kind,'added');assert.deepEqual(m.hunks.map(h=>h.row),[0,6]);
});
test('continuous windows and semantic pages independently cover all 20000 rows within DOM bounds',()=>{
 const m=parseDiff(`@@ -0,0 +1,19999 @@\n${Array.from({length:19999},(_,i)=>`+line ${i}`).join('\n')}`);
 assert.equal(m.rows.length,20000);
 const paged=[];for(let page=0;page<100;page++){const r=visibleRange(0,360,24,m.rows.length,true,page);assert.ok(r.end-r.start<=200);paged.push(...m.rows.slice(r.start,r.end));}
 assert.deepEqual(paged,m.rows);
 for(const rowHeight of [24,48,72]){
  const seen=new Set();for(let top=0;top<20000*rowHeight;top+=360){const r=visibleRange(top,360,rowHeight,m.rows.length,false,0);assert.ok(r.end-r.start<=400);for(let i=r.start;i<r.end;i++)seen.add(i);}
  assert.equal(seen.size,20000);
 }
 const giant=visibleRange(0,50000,24,20000,false,0);assert.equal(giant.end-giant.start,400);
});
