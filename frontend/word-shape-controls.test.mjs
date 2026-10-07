import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import vm from "node:vm";
function mount(invoke) {
  const ids = new Map(), rendered = [], events = {};
  for (const id of ["word-dictionary-form","word-dictionary-query","word-dictionary-language","word-dictionary-status","word-dictionary-result","word-dictionary-search","word-dictionary-prev","word-dictionary-next","word-replay-id","word-replay-show","word-replay-status","word-replay-result","joe-passage","joe-language"]) {
    ids.set(id,{value:"",children:[],handlers:{},addEventListener(type,fn){this.handlers[type]=fn;},replaceChildren(...children){this.children=children;}});
  }
  ids.get("word-dictionary-language").value="eng"; ids.get("word-dictionary-query").value="bank";
  const state={selectedSession:"thread-1"};
  const document={getElementById:id=>ids.get(id),addEventListener:(name,fn)=>{events[name]=fn;}};
  vm.runInNewContext(fs.readFileSync(new URL("./word-shape-controls.js",import.meta.url),"utf8"),{document,state,invoke,BombWordShapes:{render:(parent,value)=>rendered.push(value),renderDictionary:(parent,value)=>rendered.push(value)}});
  return {ids,rendered,state,events};
}
const submit = ui=>ui.ids.get("word-dictionary-form").handlers.submit({preventDefault(){}});
const ready = {schema:"bomb-code/dictionary-shapes/v1",status:"ready",resultCount:22,hits:Array.from({length:20},()=>({senseId:"s"}))};
test("dictionary paging stays local and retains exact query/language",async()=>{
  const calls=[];const ui=mount(async(command,args)=>{calls.push({command,args});return ready;});
  await submit(ui); await new Promise(r=>setImmediate(r));
  assert.equal(calls[0].command,"word_shape_dictionary");assert.equal(calls[0].args.payload.query,"bank");assert.equal(calls[0].args.payload.language,"eng");
  ui.ids.get("word-dictionary-next").handlers.click();await new Promise(r=>setImmediate(r));
  assert.equal(calls[1].args.payload.offset,20);assert.equal(ui.rendered.length,2);
});
test("edited query rejects late source results",async()=>{
  let resolve; const ui=mount(()=>new Promise(r=>{resolve=r;}));submit(ui);
  ui.ids.get("word-dictionary-query").value="月";ui.ids.get("word-dictionary-query").handlers.input();resolve(ready);await new Promise(r=>setImmediate(r));
  assert.equal(ui.rendered.length,0);assert.equal(ui.ids.get("word-dictionary-next").disabled,true);
});
test("malformed response clears prior output and cannot enable extra pages",async()=>{
  const ui=mount(async()=>({...ready,resultCount:Infinity}));submit(ui);await new Promise(r=>setImmediate(r));
  assert.equal(ui.rendered.length,0);assert.match(ui.ids.get("word-dictionary-status").textContent,/unavailable/);
});
const id="ded805e9-e006-4125-bad7-48b9191b35b4";
const replay={schema:"bomb-code/word-shape-replay/v1",requestId:id,threadId:"thread-1",wordShapes:{status:"ready"},authority:{toolsDispatched:false,approvalsGranted:false,memoryCommitted:false},notice:"No provider call"};
test("saved review inspection has no provider call and no drafting side effect",async()=>{
  const calls=[];const ui=mount(async(command,args)=>{calls.push({command,args});return replay;});ui.ids.get("word-replay-id").value=id;
  await ui.ids.get("word-replay-show").handlers.click();assert.equal(calls[0].command,"joe_word_shape_replay");assert.equal(ui.rendered.length,1);
});
test("thread and context edits reject pending saved review",async()=>{
  for(const event of ["bomb-code:thread-selected","bomb-code:joe-interpretation"]){
    let resolve;const ui=mount(()=>new Promise(r=>{resolve=r;}));ui.ids.get("word-replay-id").value=id;
    const pending=ui.ids.get("word-replay-show").handlers.click();ui.events[event]({detail:{status:"invalidated"}});resolve(replay);await pending;assert.equal(ui.rendered.length,0);
  }
});
test("saved review boundary violations and invalid UUIDs are withheld",async()=>{
  let calls=0;const ui=mount(async()=>{calls++;return {...replay,authority:{toolsDispatched:true}};});
  ui.ids.get("word-replay-id").value="../escape";await ui.ids.get("word-replay-show").handlers.click();assert.equal(calls,0);
  ui.ids.get("word-replay-id").value=id;await ui.ids.get("word-replay-show").handlers.click();assert.equal(calls,1);assert.equal(ui.rendered.length,0);
});

test("selected memory is validated again before saved shapes are displayed",async()=>{
  const calls=[];const ui=mount(async(command)=>{calls.push(command);if(command==="memory_recall")throw new Error("source changed");return {...replay,memoryReceiptId:id};});
  ui.ids.get("word-replay-id").value=id;await ui.ids.get("word-replay-show").handlers.click();
  assert.deepEqual(calls,["joe_word_shape_replay","memory_recall"]);assert.equal(ui.rendered.length,0);
});

test("source validation cannot display shapes into a changed thread",async()=>{
  let resolve;const ui=mount(async(command)=>command==="memory_recall"?new Promise(r=>{resolve=r;}):{...replay,memoryReceiptId:id});
  ui.ids.get("word-replay-id").value=id;const pending=ui.ids.get("word-replay-show").handlers.click();
  await new Promise(r=>setImmediate(r));ui.state.selectedSession="thread-2";resolve({status:"ready"});await pending;assert.equal(ui.rendered.length,0);
});
