import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { createContext, runInContext } from 'node:vm';

// Exercise the actual controller's publication/retry path, including a failed DOM render.
const keys = ['planner','plan_approval','implementer','auditor','verifier','acceptance'];
const route = {backend:'codex',model:null};
const record = {
  id:'build-a',round:0,status:'implementing',queue_state:'reserved',cleanup_pending:true,
  active_session_id:'impl-session',active_session_role:'implementer',active_session_round:0,
  spec:{objective:'<img src=x onerror=alert(1)>',project_root:'/fixture',write_set:['greeting.py'],max_repairs:2,roles:Object.fromEntries(['planner','implementer','auditor','verifier'].map(k=>[k,route]))},
  dependencies:[],plan:'PLAN',steps:[{role:'planner',round:0,output:'PLAN',session_id:'planner-session'}],
  progress:{basis:'workflow_checkpoints',completed:2,total:6,percent:33,round:0,checkpoints:keys.map((key,i)=>({key,label:key,completed:i<2,session_id:i===0?'planner-session':null}))}
};
let response = [record], failRender = false;
const elements = new Map();
function element(id) {
  if (!elements.has(id)) {
    let html = '';
    elements.set(id, {
      id,value:'',selectedOptions:[],scrollLeft:0,scrollTop:0,
      classList:{contains:()=>id==='view-builds',add(){},remove(){},toggle(){}},
      querySelectorAll:()=>[],
      get innerHTML(){return html;},
      set innerHTML(value){if(id==='collaboration-graph' && failRender){failRender=false;throw Error('simulated rendering failure');}html=value;}
    });
  }
  return elements.get(id);
}
const sandbox = {
  window:{BombBuildsHost:{refreshActivity(){},async openSession(){}},addEventListener(){}},
  document:{activeElement:null,hidden:false,addEventListener(){}},
  $:element,escapeHtml:value=>String(value).replaceAll('&','&amp;').replaceAll('<','&lt;').replaceAll('>','&gt;').replaceAll('"','&quot;'),
  invoke:async command=>command==='list_builds'?structuredClone(response):command==='list_sessions'?[]:2,
  setInterval(){},Date,console
};
const context = createContext(sandbox);
for (const name of ['collaboration.js','builds.js']) runInContext(readFileSync(new URL(name,import.meta.url),'utf8'),context);
await new Promise(setImmediate);
const api = sandbox.window.BombBuilds;
assert.equal(api.sessionSummary('impl-session').fresh,true);
assert.equal(api.sessionSummary('impl-session').text,'33% workflow checkpoints · 2/6 completed');
assert.ok(element('collaboration-graph').innerHTML.includes('&lt;img'), 'untrusted objectives remain text');
assert.ok(!element('collaboration-graph').innerHTML.includes('<img'), 'no markup injection');

const accepted = structuredClone(record);
accepted.status='accepted'; accepted.active_session_id=null; accepted.active_session_role=null; accepted.active_session_round=null;
accepted.steps.push({role:'implementer',round:0,output:'Done',session_id:'impl-session'});
accepted.progress.completed=6;accepted.progress.percent=100;accepted.progress.checkpoints.forEach(c=>{c.completed=true;});
response=[accepted];failRender=true;
await api.refresh();
assert.equal(api.sessionSummary('impl-session').fresh,false,'failed render cannot publish a fresh snapshot');
assert.equal(api.sessionSummary('impl-session').text,'33% workflow checkpoints · 2/6 completed','last-good state survives rendering failure');
assert.ok(element('collaboration-snapshot').textContent.includes('Refresh failed'));
await api.refresh();
assert.equal(api.sessionSummary('impl-session').fresh,true,'identical retry re-renders and recovers');
assert.equal(api.sessionSummary('impl-session').text,'100% workflow checkpoints · 6/6 completed');
sandbox.document.activeElement = element('builds-dependencies');
const changed = structuredClone(accepted); changed.spec.objective = 'New prerequisite label'; response=[changed];
await api.refresh();
assert.ok(!element('builds-dependencies').innerHTML.includes('New prerequisite label'), 'focused selector keeps the user selection stable');
sandbox.document.activeElement = null;
await api.refresh();
assert.ok(element('builds-dependencies').innerHTML.includes('New prerequisite label'), 'blur retries the deferred update even for an identical response');
response=[null];await api.refresh();
assert.equal(api.sessionSummary('impl-session').fresh,false,'malformed host response cannot be current');
assert.equal(api.sessionSummary('impl-session').text,'100% workflow checkpoints · 6/6 completed','malformed response preserves last good data');
console.log('builds.test.mjs: actual refresh rollback/retry, malformed data and escaping passed');
