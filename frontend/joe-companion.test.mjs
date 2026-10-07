import test from 'node:test';
import assert from 'node:assert/strict';
import {createRequire} from 'node:module';
import fs from 'node:fs';
const require = createRequire(import.meta.url), Joe = require('./joe-companion.js');
const source = (rows = [], extra = {}) => ({ selectedSession: 'a', sessions: [{id:'a', live:true}], transcriptLoaded: new Set(['a']), transcriptBySession: new Map([['a',rows]]), ...extra });
test('bounded export contains only selected user/agent/plan excerpts and explicit outcome', () => {
  const rows = [{role:'thought',body:'PRIVATE_THOUGHT'}, {role:'term',body:'RAW_SECRET'}, ...Array.from({length:12},(_,i)=>({role:i%2?'agent':'user',body:`message ${i} `+'😀'.repeat(1500),at:String(i)}))];
  const state=source(rows); state.transcriptBySession.set('b',[{role:'user',body:'OTHER_THREAD'}]);
  const s=Joe.snapshot(state,'User selected outcome'), passage=Joe.prepare(s);
  assert.equal(s.rows.length,8); assert.equal(s.omittedRows,6); assert.equal(s.clippedRows,8);
  assert.ok(passage.length<=12000); assert.ok(passage.includes('User selected outcome'));
  for (const forbidden of ['PRIVATE_THOUGHT','RAW_SECRET','OTHER_THREAD']) assert.ok(!passage.includes(forbidden));
  assert.ok(!s.rows.some(r=>/[\uD800-\uDBFF]$/.test(r.body)));
});
test('unknown outcome and incomplete history cannot become an alignment claim', () => {
  const s=Joe.snapshot(source([{role:'agent',body:'All done.'}]));
  assert.ok(s.notices.some(n=>n.includes('alignment is unknown')));
  assert.throws(()=>Joe.prepare(Joe.snapshot(source([], {transcriptLoaded:new Set()}))));
  assert.throws(()=>Joe.prepare(Joe.snapshot(source([], {selectedSession:null}))));
});
test('cues are observations; pending approvals require a live session and exact metadata', () => {
  const rows=[{role:'agent',body:'Which target?'},{role:'approval',body:'request',meta:{requestId:'p1'}},{role:'plan',body:'- [pending] tests'}];
  const s=Joe.snapshot(source(rows,{tools:[{sessionId:'a',status:'failed'},{sessionId:'b',status:'failed'}]}),'Goal');
  assert.ok(s.notices.some(n=>n.includes('contains a question')));
  assert.ok(s.notices.some(n=>n.includes('1 native approval')));
  assert.ok(s.notices.some(n=>n.includes('1 recent tool failure')));
  assert.ok(s.notices.some(n=>n.includes('pending entries')));
  assert.ok(!Joe.snapshot(source(rows,{sessions:[{id:'a',live:false}]}),'Goal').notices.some(n=>n.includes('native approval')));
});
test('review binding changes with same-thread edits, outcome or recorded build checkpoints', () => {
  const state=source([{role:'user',body:'request'}],{transcriptRevisionBySession:new Map([['a',1]])});
  const s=Joe.snapshot(state,'Goal',{id:'build',round:0,text:'33%'});
  state.transcriptRevisionBySession.set('a',2);
  assert.notEqual(s.binding,Joe.snapshot(state,'Goal',{id:'build',round:0,text:'33%'}).binding);
  assert.notEqual(s.binding,Joe.snapshot(state,'Other goal',{id:'build',round:0,text:'33%'}).binding);
  assert.notEqual(s.binding,Joe.snapshot(state,'Goal',{id:'build',round:0,text:'50%'}).binding);
});
test('cube travel avoids protected rectangles and stays still while typing', () => {
  const blocked = Joe.placeCube({ x: 20, y: 400 }, { x: 100, y: 80, width: 180, height: 90 }, [{ x: 0, y: 0, width: 960, height: 640 }], { width: 960, height: 640 }, 128);
  assert.equal(blocked.travel, false);
  assert.equal(blocked.fallback, true);
  const clear = Joe.placeCube({ x: 640, y: 420 }, { x: 40, y: 40, width: 180, height: 80 }, [{ x: 0, y: 560, width: 960, height: 80 }], { width: 960, height: 640 }, 128);
  assert.equal(clear.travel, true);
  assert.ok(clear.y + 128 <= 560);
  assert.deepEqual(Joe.stepCube({ x: 10, y: 10 }, { x: 300, y: 10 }, 1000, { typing: true }), { x: 10, y: 10, moving: false });
  const moved = Joe.stepCube({ x: 10, y: 10 }, { x: 300, y: 10 }, 1000, {});
  assert.ok(moved.x - 10 <= 320 * 0.05 + 1e-9);
  assert.equal(moved.moving, true);
});
test('gesture uses resting wings and never travel frames; pause selects neutral', () => {
  const atlas=JSON.parse(fs.readFileSync(new URL('./assets/joe/wizard-joe-hd.json',import.meta.url)));
  assert.deepEqual(Joe.gestureFrame(atlas,350,true),atlas.frames['wing-adjust'].frame);
  assert.deepEqual(Joe.gestureFrame(atlas,650,true,true),atlas.frames['idle-neutral'].frame);
  assert.deepEqual(Joe.gestureFrame(atlas,2000,true),atlas.frames['idle-neutral'].frame);
});
function mount() {
  const all=[]; class El {
    constructor(tag,doc) {this.tag=tag;this.ownerDocument=doc;this.children=[];this.handlers={};this.style={};this.dataset={};this.attrs={};this.hidden=false;this.value='';all.push(this);}
    appendChild(child) {child.parent=this;this.children.push(child);return child;}
    before(marker) {marker.parent=this.parent;this.parent.children.push(marker);}
    replaceWith(node) {this.parent.appendChild(node);}
    remove() {this.removed=true;}
    replaceChildren() {this.children=[];}
    setAttribute(k,v) {this.attrs[k]=v;}
    addEventListener(k,v) {this.handlers[k]=v;}
    removeEventListener(k) {delete this.handlers[k];}
    focus() {this.focused=true;}
    getContext() {return {clearRect(){},fillText(){},drawImage(){}};}
  }
  const store=new Map(), win={performance:{now:()=>40000},localStorage:{getItem:k=>store.get(k),setItem:(k,v)=>store.set(k,v)},matchMedia:()=>({matches:false,addEventListener(){},removeEventListener(){}}),setInterval:()=>1,clearInterval(){},requestAnimationFrame:()=>1,cancelAnimationFrame(){},fetch:()=>new Promise(()=>{})};
  const doc=new El('document');doc.defaultView=win;doc.createElement=tag=>new El(tag,doc);doc.createComment=()=>new El('comment',doc);doc.body=new El('body',doc);const original=new El('details',doc);original.id='wizard-joe';doc.body.appendChild(original);doc.getElementById=id=>all.find(x=>x.id===id);
  const state=source([{role:'user',body:'bounded request',at:'1'}],{transcriptRevisionBySession:new Map([['a',1]])});const calls=[];let validator;
  const guide={setContextValidator:fn=>{validator=fn;},setPassage:(sentence)=>calls.push({type:'prepare',sentence}),invalidateContext:()=>calls.push({type:'invalidate'})};
  const ui=Joe.attach({document:doc,getState:()=>state,guide});return{all,doc,state,calls,ui,original,validator:()=>validator};
}
test('actual companion is body-level; click is local; same-thread changes invalidate prepared review', () => {
  const m=mount();assert.equal(m.ui.element.parent,m.doc.body);assert.equal(m.calls.length,0);
  const button=m.all.find(x=>x.attrs['aria-controls']==='joe-companion-drawer');button.handlers.click();
  assert.equal(button.attrs['aria-expanded'],'true');assert.equal(m.calls.length,0);
  m.all.find(x=>x.textContent==='Prepare thread review').handlers.click();
  assert.equal(m.calls.length,1);assert.equal(m.calls[0].type,'prepare');const sentence=m.calls[0].sentence;
  assert.equal(m.validator()(sentence),true);
  m.state.transcriptRevisionBySession.set('a',2);
  assert.equal(m.validator()(sentence),false); // Reject even before the one-second observer refresh.
  m.ui.refresh();assert.ok(m.calls.some(x=>x.type==='invalidate'));
  assert.ok(m.all.some(x=>x.textContent==='Thread context changed since the previous review.'));
  m.ui.destroy();assert.equal(m.ui.element.removed,true);
});

test('an older pending plan does not override a newer completed plan', () => {
 const s=Joe.snapshot(source([{role:'plan',body:'- [pending] tests'},{role:'plan',body:'- [completed] tests'}]),'Goal');
 assert.ok(!s.notices.some(n=>n.includes('pending entries')));
});
