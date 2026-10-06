import test from 'node:test';
import assert from 'node:assert/strict';
import {createRequire} from 'node:module';
const require = createRequire(import.meta.url);
const faces = require('./spatial-faces.js');
const source = () => ({selectedSession:'a',sessions:[{id:'a',label:'Native A',backend:'codex'},{id:'b',label:'<img src=x onerror=run()>',backend:'claude',cwd:'/actual/project'},{id:'c',label:'Unloaded saved thread',backend:'grok'}],explainBySession:new Map([['a',[{text:'A explanation'}]],['b',[{text:'Exact B explanation'}]]]),transcriptBySession:new Map([['a',[{role:'user',body:'Actual A body'}]],['b',[{role:'user',body:'<script>privateB()</script>'},{role:'agent',body:'B agent answer'},{role:'term',body:'hidden ACP noise'}]]]),transcriptLoaded:new Set(['a','b'])});

test('native transcript body and exact per-thread narration are retained; technical noise is not prose',()=>{
 const state=source(), view=faces.threadView(state,'b',{sessions:[{id:'b',savedOnly:true,phase:'tools'}]});
 assert.equal(view.title,state.sessions[1].label); assert.equal(view.narration[0].text,'Exact B explanation');
 assert.deepEqual(view.transcript.map(e=>e.text),['<script>privateB()</script>','B agent answer']);
 assert.equal(view.status,'Saved conversation'); assert.equal(view.project,'/actual/project');
 assert.equal(faces.threadView(state,'missing'),null);
 assert.equal(faces.threadView(state,'c').cached,false);
});
test('excerpts expose their limits and never invent missing thread content',()=>{
 const state=source(); state.transcriptBySession.set('b',Array.from({length:15},(_,i)=>({role:'agent',body:i===14?'x'.repeat(8001):String(i)})));
 const view=faces.threadView(state,'b'); assert.equal(view.transcriptCount,15); assert.equal(view.transcript.length,10);
 assert.match(view.transcript[9].text,/Preview excerpt/); assert.equal(view.status,'Activity status unavailable');
 assert.deepEqual(faces.threadView(state,'c').narration,[]);
});
test('face bounds preserve scene containment even when smaller than preferred minimums',()=>{
 for(const bounds of [{width:1000,height:700},{width:530,height:660},{width:40,height:40}]) for(const rect of [{x:-100,y:-99,width:9999,height:9999},{x:9999,y:9999,width:10,height:10}]) {
  const actual=faces.boundedRect(rect,bounds);
  assert.ok(actual.x>=0&&actual.y>=0); assert.ok(actual.x+actual.width<=bounds.width); assert.ok(actual.y+actual.height<=bounds.height);
 }
 const bounds={width:1000,height:700}, occupied=[{x:12,y:155,width:400,height:400}];
 const next=faces.previewRect(bounds,occupied); assert.ok(next.x>=422); assert.ok(next.y>=155);
});

function mount(){
 const created=[], masks=[], selectedCalls=[]; let workspaceRect={x:16,y:155,width:460,height:445};
 const state=source();
 class Element {
  constructor(tag='div'){this.tag=tag;this.children=[];this.listeners={};this.style={};this.dataset={};this.attributes={};this.value='';this.textContent='';this.clientWidth=1000;this.clientHeight=700;this.classList={add(){},remove(){}};created.push(this);}
  appendChild(child){child.parent=this;this.children.push(child);return child;}
  insertBefore(child,before){child.parent=this;this.children.splice(this.children.indexOf(before),0,child);return child;}
  replaceChildren(...children){this.children=children;children.forEach(e=>e.parent=this);}
  remove(){if(this.parent)this.parent.children=this.parent.children.filter(e=>e!==this);this.parent=null;}
  setAttribute(key,value){this.attributes[key]=value;}
  addEventListener(type,handler){(this.listeners[type]??=new Set()).add(handler);}
  removeEventListener(type,handler){this.listeners[type]?.delete(handler);}
  dispatch(type,event={}){for(const handler of this.listeners[type]||[])handler({...event,type});}
  setPointerCapture(){}
  getBoundingClientRect(){return{left:0,top:0,width:1000,height:700};}
  set innerHTML(_){throw new Error('Unsafe HTML sink');}
 }
 const doc=new Element('document'); doc.createElement=tag=>{const e=new Element(tag);e.ownerDocument=doc;return e;};
 const win={ResizeObserver:class{observe(){}disconnect(){}},setInterval:()=>1,clearInterval(){},queueMicrotask:fn=>fn()};doc.defaultView=win;
 const root=doc.createElement('root'), workspace=doc.createElement('native-workspace'), composer=doc.createElement('textarea');composer.value='Original unsent native draft';workspace.appendChild(composer);root.appendChild(workspace);
 const scene={element:root,workspace,getWorkspaceRect:()=>({...workspaceRect}),setWorkspaceRect:rect=>{workspaceRect={...rect};return rect;},setExclusionElements:(elements,owner)=>masks.push({target:'scene',elements:[...elements],owner}),refreshExclusions(){}};
 const host={scene,snapshot:()=>({sessions:[{id:'a',phase:'idle'},{id:'b',savedOnly:true}]}),setView:view=>{root.dataset.view=view;}};
 const ui=faces.attach({host,background:{setExclusionElements:(elements,owner)=>masks.push({target:'background',elements:[...elements],owner}),refreshExclusions(){}},getState:()=>state,selectSession:async id=>{selectedCalls.push(id);state.selectedSession=id;doc.dispatch('bomb-code:thread-selected');},activateView:()=>{}});
 return{ui,state,created,root,workspace,composer,doc,masks,selectedCalls,rect:()=>workspaceRect};
}
function panel(m,id){return m.created.find(node=>node.tag==='article'&&node.dataset.threadId===id&&node.parent);}
function descendants(node){return[node,...node.children.flatMap(descendants)];}

test('opening is read-only and renders literal cached B context while the original composer remains A',()=>{
 const m=mount();m.ui.openFace('b');
 assert.deepEqual(m.selectedCalls,[]);assert.equal(m.state.selectedSession,'a');assert.equal(m.composer.value,'Original unsent native draft');
 const nodes=descendants(panel(m,'b'));assert.ok(nodes.some(node=>node.textContent==='<script>privateB()</script>'));
 assert.ok(nodes.some(node=>node.textContent==='Exact B explanation'));
 assert.equal(m.created.filter(n=>n.tag==='textarea').length,1);
 for(const target of ['scene','background']) assert.ok(m.masks.some(mask=>mask.target===target&&mask.owner==='secondary-faces'&&mask.elements.includes(panel(m,'b'))));
 m.ui.destroy(); assert.ok(m.masks.slice(-2).every(mask=>mask.elements.length===0));
});
test('Focus promotes preview geometry and demotes prior native thread without duplicating coding controls',async()=>{
 const m=mount(), previous=m.rect();m.ui.openFace('b');const b=panel(m,'b');const next={x:parseFloat(b.style.left),y:parseFloat(b.style.top),width:parseFloat(b.style.width),height:parseFloat(b.style.height)};
 await m.ui.promote('b');assert.deepEqual(m.selectedCalls,['b']);assert.equal(m.state.selectedSession,'b');assert.deepEqual(m.rect(),next);
 assert.equal(panel(m,'b'),undefined);const a=panel(m,'a');assert.ok(a);assert.equal(parseFloat(a.style.left),previous.x);
 assert.ok(descendants(a).some(n=>n.textContent==='Actual A body'));
 assert.equal(m.composer.parent,m.workspace);assert.equal(m.created.filter(n=>n.tag==='textarea').length,1);
 m.ui.destroy();
});
test('keyboard movement and resizing are bounded and do not intercept composer keys',()=>{
 const m=mount();m.ui.openFace('b');const nodes=descendants(panel(m,'b')),move=nodes.find(n=>n.className==='spatial-face-move'),resize=nodes.find(n=>n.className==='spatial-face-resize');
 let prevented=0;move.dispatch('keydown',{key:'ArrowRight',preventDefault(){prevented++;}});
 assert.equal(prevented,1);const after=panel(m,'b').style.left;
 move.dispatch('keydown',{key:'ArrowLeft',ctrlKey:true,preventDefault(){prevented++;}});assert.equal(panel(m,'b').style.left,after);
 resize.dispatch('keydown',{key:'ArrowRight',shiftKey:true,preventDefault(){prevented++;}});assert.ok(parseFloat(panel(m,'b').style.width)>=320);
 assert.equal(m.composer.listeners.keydown,undefined);m.ui.destroy();
});
test('external selection and cache hydration update retained thread ownership; closing keeps native threads',()=>{
 const m=mount();m.ui.openFace('c');assert.ok(descendants(panel(m,'c')).some(n=>/not loaded/.test(n.textContent)));
 m.state.transcriptBySession.set('c',[{role:'agent',body:'Hydrated original C message'}]);m.state.transcriptLoaded.add('c');m.ui.refresh();
 assert.ok(descendants(panel(m,'c')).some(n=>n.textContent==='Hydrated original C message'));
 m.state.selectedSession='c';m.doc.dispatch('bomb-code:thread-selected');assert.ok(panel(m,'a'));assert.equal(panel(m,'c'),undefined);
 m.ui.openFace('b');const close=descendants(panel(m,'b')).find(n=>n.attributes['aria-label']?.startsWith('Close'));close.dispatch('click');
 assert.equal(panel(m,'b'),undefined);assert.equal(m.state.sessions.length,3);assert.deepEqual(m.selectedCalls,[]);m.ui.destroy();
});
test('Arrange tiles two faces without overlap in sufficient scene bounds and uses the real primary API',()=>{
 const m=mount();m.ui.openFace('b');m.ui.arrange();const a=m.rect(),b=panel(m,'b');
 assert.ok(a.x+a.width<parseFloat(b.style.left));assert.ok(a.y>=155);assert.ok(a.height>=240);
 m.ui.destroy();
});
test('Arrange stacks two real faces in a narrow scene without moving native typing ownership',()=>{
 const m=mount();m.root.clientWidth=530;m.root.clientHeight=660;m.ui.openFace('b');m.ui.arrange();
 const primary=m.rect(),secondary=panel(m,'b');
 assert.ok(primary.y+primary.height<parseFloat(secondary.style.top));
 assert.ok(parseFloat(secondary.style.top)+parseFloat(secondary.style.height)<=660);
 assert.equal(m.state.selectedSession,'a');assert.deepEqual(m.selectedCalls,[]);m.ui.destroy();
});
