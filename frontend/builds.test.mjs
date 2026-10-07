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
  invoke:async (command, args)=>{
  if(command==='list_builds') return structuredClone(response);
  if(command==='list_sessions') return [];
  if(command==='get_build_concurrency'||command==='set_build_concurrency') return 2;
  if(command==='preview_build'){
    return {dry_run:true,would_persist:false,would_spawn_agents:false,project_root:args.spec.project_root,repository:'/repo',head_commit:'abc123',write_set:args.spec.write_set,objective:args.spec.objective,max_repairs:args.spec.max_repairs,roles:[{role:'planner',backend:'grok',model:null}],dependencies:[],predicted_queue_state:'queued',concurrency_limit:2,clean_working_tree:true,notes:['Dry run only: nothing was saved and no native agents were started.']};
  }
  return null;
},
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

// Dry-run preview path: fills form fields and clicks Preview without create_build.
element('builds-project').value = '/fixture';
element('builds-objective').value = 'preview only';
element('builds-scope').value = 'src\ntests';
element('builds-repairs').value = '2';
for (const role of ['planner','implementer','auditor','verifier']) {
  element(`builds-${role}-engine`).value = 'grok';
  element(`builds-${role}-model`).value = '';
}
element('builds-dependencies').selectedOptions = [];
let created = false;
const prevInvoke = sandbox.invoke;
sandbox.invoke = async (command, args) => {
  if (command === 'create_build') { created = true; throw new Error('create must not run during dry-run'); }
  return prevInvoke(command, args);
};
await element('builds-preview').onclick();
assert.equal(created, false, 'dry-run must not create a build');
assert.equal(element('builds-preview-panel').hidden, false, 'preview panel visible');
assert.ok(element('builds-preview-panel').innerHTML.includes('Dry-run preview'));
assert.ok(element('builds-preview-panel').innerHTML.includes('No agents were started'));
assert.ok(element('builds-status').textContent.includes('Dry run ok'));
console.log('builds.test.mjs: dry-run preview path passed');

element('builds-objective').value = 'planner: grok\nimplementer: claude / claude-opus-4.5\nauditor: claude code\nverifier: codex / gpt-5.4\nnot a role: nope';
element('builds-objective').oninput();
assert.equal(element('builds-planner-engine').value, 'grok');
assert.equal(element('builds-implementer-engine').value, 'claude');
assert.equal(element('builds-implementer-model').value, 'claude-opus-4.5');
assert.equal(element('builds-auditor-engine').value, 'claude');
assert.equal(element('builds-verifier-model').value, 'gpt-5.4');
assert.match(element('builds-bindings').innerHTML, /Implementer · claude · claude-opus-4\.5/);
assert.match(element('builds-bindings').innerHTML, /Auditor · claude · configured default/);
console.log('builds.test.mjs: spoken role lines set the engines before submit');

element('builds-objective').value = `Ship the desk.
1. Parse a numbered plan into parts
Each part is its own reviewed build.
2. Queue the parts in order
The next part waits for acceptance.
planner: grok
implementer: claude / claude-opus-4.5`;
element('builds-objective').oninput();
assert.equal(element('builds-sequence').hidden, false);
assert.match(element('builds-sequence').innerHTML, /Parse a numbered plan into parts/);
assert.match(element('builds-sequence').innerHTML, /Starts after you accept part 1/);
assert.equal(element('builds-implementer-model').value, 'claude-opus-4.5');
assert.match(element('builds-submit-help').textContent, /queues 2 builds/);
assert.equal(element('builds-create').textContent, 'Queue 2 builds');
let previewObjective = '';
let previewDependencies = null;
const sequenceInvoke = sandbox.invoke;
sandbox.invoke = async (command, args) => {
  if (command === 'preview_build') {
    previewObjective = args.spec.objective;
    previewDependencies = Array.from(args.dependencies);
  }
  if (command === 'create_build') throw new Error('preview must not create');
  return sequenceInvoke(command, args);
};
await element('builds-preview').onclick();
assert.match(previewObjective, /Ship the desk/);
assert.match(previewObjective, /Parse a numbered plan/);
assert.doesNotMatch(previewObjective, /planner: grok/);
assert.deepEqual(previewDependencies, []);
assert.match(element('builds-preview-panel').innerHTML, /Sequence · 2 parts/);
assert.match(element('builds-preview-panel').innerHTML, /No agents were started/);
assert.match(element('builds-status').textContent, /nothing submitted/);
assert.match(element('builds-status').textContent, /starts the first planner/);
console.log('builds.test.mjs: a numbered plan previews as a sequence and starts nobody');

element('builds-scope').value = 'greeting.py';
element('builds-objective').value = '1. Rename the greeting in greeting.py\nCheck it.\n2. Add one sentence to README.md\nCheck the sentence.';
element('builds-objective').oninput();
assert.match(element('builds-sequence').innerHTML, /README.md is not a write path/);
assert.doesNotMatch(element('builds-sequence').innerHTML, /greeting.py is not a write path/);
await element('builds-preview').onclick();
assert.match(element('builds-preview-panel').innerHTML, /README.md is not a write path/);
assert.match(element('builds-status').textContent, /README.md is not a write path/);
console.log('builds.test.mjs: a later part names a file outside the write paths');

element('builds-scope').value = 'greeting.py\nREADME.md';
element('builds-objective').value = 'I opened this folder because the greeting a person sees is wrong.\n1. Fix the spelling in greeting.py\nDo not edit test_greeting.py to force a pass. python3 -m unittest -v is the check.\n2. Rewrite the README opening after I accept that result\nREADME.md should tell a new reader that greet says Hello.';
element('builds-objective').oninput();
assert.match(element('builds-sequence').innerHTML, /Fix the spelling in greeting.py/);
assert.match(element('builds-sequence').innerHTML, /Rewrite the README opening/);
assert.doesNotMatch(element('builds-sequence').innerHTML, /test_greeting.py is not a write path/);
assert.doesNotMatch(element('builds-sequence').innerHTML, /greeting.py is not a write path/);
assert.doesNotMatch(element('builds-sequence').innerHTML, /README.md is not a write path/);
console.log('builds.test.mjs: a realistic note can name a test file without making it a write path');

element('builds-objective').value = `Ship the desk.
1. Parse a numbered plan into parts
Each part is its own reviewed build.
2. Queue the parts in order
The next part waits for acceptance.
planner: grok
implementer: claude / claude-opus-4.5`;
element('builds-objective').oninput();
const calls = [];
sandbox.invoke = async (command, args) => {
  if (command === 'create_build') {
    const id = `part-${calls.length + 1}`;
    calls.push({ id, objective: args.spec.objective, dependencies: Array.from(args.dependencies) });
    return { id, status: 'planning' };
  }
  return sequenceInvoke(command, args);
};
await element('builds-form').onsubmit({ preventDefault() {} });
assert.equal(calls.length, 2);
assert.match(calls[0].objective, /Parse a numbered plan/);
assert.deepEqual(calls[0].dependencies, []);
assert.match(calls[1].objective, /Queue the parts in order/);
assert.deepEqual(calls[1].dependencies, ['part-1']);
assert.match(element('builds-status').textContent, /Queued 2 builds/);
assert.match(element('builds-status').textContent, /Approve each plan/);
console.log('builds.test.mjs: submit queues the parts and chains acceptance');

element('builds-project').value = '';
element('builds-scope').value = '';
await element('builds-preview').onclick();
assert.equal(element('builds-setup').open, true);
assert.match(element('builds-status').textContent, /one write path/);
assert.match(element('builds-status').textContent, /starts nobody/);
element('builds-objective').value = '## Draw the lattice\nHang the sheets.\n## Name the corners\nEach corner is one control.';
element('builds-objective').oninput();
assert.match(element('builds-sequence').innerHTML, /Draw the lattice/);
assert.match(element('builds-sequence').innerHTML, /Name the corners/);
assert.match(element('builds-sequence').innerHTML, /Starts after you accept part 1/);
assert.equal(element('builds-create').textContent, 'Queue 2 builds');
console.log('builds.test.mjs: headings split the same way, and a preview without a project opens the setup');

element('builds-status').textContent = 'Dry run ok — 2 parts, nothing submitted.';
response = [null];
await api.refresh();
assert.match(element('builds-status').textContent, /nothing submitted/);
assert.match(element('collaboration-snapshot').textContent, /Refresh failed/);
console.log('builds.test.mjs: a failed refresh keeps the preview message');
