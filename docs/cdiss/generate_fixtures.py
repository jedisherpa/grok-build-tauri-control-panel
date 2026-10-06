"""Authored grammar fixtures bound by the actual frozen source map; no provider calls.
Outputs full native binding objects. Fixture authored choices are not linguistic gold.
"""
import argparse, copy, hashlib, json, sys
from pathlib import Path
REFERENCE = Path('/Users/paulcooper/Documents/Codex/2026-10-06/tak/work/sensesnap-reference/current')
REPO = Path(__file__).resolve().parents[2]
sys.path.insert(0,str(REFERENCE))
from semantic_e8.interpretation import prepare_request, ground_outline, bind_interpretation, SELECTION_VERSION, CONTEXT_SELECTION_VERSION
from semantic_e8.tests.test_interpretation import outline
G=json.loads((REFERENCE/'semantic_e8/outputs/aligned_graph.json').read_text())
M=json.loads((REFERENCE/'semantic_e8/outputs/model.json').read_text())
MANIFEST=hashlib.sha256((REFERENCE/'round_trip_experiment/PACKAGE_MANIFEST.json').read_bytes()).hexdigest()
CASES=[('bank','The bank approved the loan.','positive','asserted'),('passive','The loan was approved by the bank.','positive','asserted'),('negative','The bank has not approved the loan.','negative','asserted'),('conditional','If the bank approves the loan, we can begin.','positive','conditional'),('context-pin','The bank approved the loan.','positive','asserted')]
parser=argparse.ArgumentParser(description=__doc__)
parser.add_argument('--case',choices=[row[0] for row in CASES])
args=parser.parse_args()
output=REPO/'crates/grok_cdiss/tests/fixtures'
for name,sentence,polarity,modality in CASES:
    if args.case and name!=args.case: continue
    out=outline(sentence,polarity,modality)
    context={'task_anchor':'Inspect an authored financing example','provenance':'authored offline fixture; not provider output or verified human intent'}
    if name=='context-pin':
        # Deliberately conflicting authored center: retained as a contextual proposal,
        # never accepted as a new dictionary equivalence or a verified interpretation.
        context['sense_snap']={'speaker':'fixture-speaker','frame_id':'fixture-context-review','overlay':{
            'meetings':[{'id':'meet:personal:financing-request','gloss':'The project financing request','definition':'Authored private reference to the project financing request'}],
            'pins':[{'id':'pin:fixture:bank-shore','speaker':'fixture-speaker','language':'eng','lemma':'bank','meeting_id':'pwn30:09213434-n','how':'explicit','frame_id':'fixture-context-review','evidence':'AUTHORED BOUNDARY FIXTURE: retain the shoreline center beside the financial source selection; this is an intentionally conflicting test proposal'},
                    {'id':'pin:fixture:loan-request','speaker':'fixture-speaker','language':'eng','lemma':'loan','meeting_id':'meet:personal:financing-request','how':'explicit','frame_id':'fixture-context-review','evidence':'AUTHORED BOUNDARY FIXTURE: loan names this private financing request; no dictionary sense or fitted point asserted'}],
            'frames':[{'id':'fixture-context-review','goal':'Inspect fitted public and unfitted private contextual pins','goal_words':['bank','loan'],'meeting_ids':['pwn30:09213434-n','meet:personal:financing-request'],'members':['fixture-speaker']}]}}
    request=prepare_request(G,M,'eng',sentence,context)
    packet=ground_outline(G,M,request,out)
    selected=[]
    for row in packet['atoms']:
        aid=row['atom']['id']
        if aid=='bank': ids=[c['sense']['id'] for c in row['candidates'] if c['sense']['pos']=='n' and c['sense'].get('definition','').startswith('a financial institution that accepts deposits')]
        elif aid=='approve': ids=[c['sense']['id'] for c in row['candidates'] if c['sense']['pos']=='v' and c['sense'].get('definition')=='give sanction to']
        else: ids=[]
        item={'atom_id':aid,'sense_ids':ids,'reason':'Authored source-ID choice; no live interpretation quality claim'}
        if name=='context-pin' and aid in {'bank','loan'}:
            item['context_meeting_ids']=['pwn30:09213434-n' if aid=='bank' else 'meet:personal:financing-request']
        selected.append(item)
    selection={'schema':CONTEXT_SELECTION_VERSION if name=='context-pin' else SELECTION_VERSION,'readings':[{'id':'r1','sense_bindings':selected,'uncertainty':['Authored grammar fixture, not a live reading.','Loan noun is unavailable in scoped source inventory.']}],'reason':'Authored boundary test'}
    binding=bind_interpretation(G,M,request,out,selection,'offline-fixture')
    result={'schema':'bomb-code/joe-result/v1','requestId':'00000000-0000-4000-8000-'+str(CASES.index((name,sentence,polarity,modality))+1).zfill(12),'threadId':'00000000-0000-4000-8000-000000000100','sentence':sentence,'language':'eng','status':'grounded-model-proposal','provider':'authored-offline-fixture','model':'authored-grammar-not-live/v1','interpretation':{'status':'grounded-model-proposal','binding':binding},'reference':{'root':str(REFERENCE),'manifestSha256':MANIFEST},'referenceRequested':{'manifestSha256':MANIFEST},'authority':{'toolsDispatched':False,'approvalsGranted':False,'memoryCommitted':False},'clarifications':[], 'fixtureProvenance': {'kind':'authored grammar and source choices through unmodified native binder','providerCalls':0,'coordinatesGeneratedBy':'frozen actual fitted source model','sourceFiles':{'graph':hashlib.sha256((REFERENCE/'semantic_e8/outputs/aligned_graph.json').read_bytes()).hexdigest(),'model':hashlib.sha256((REFERENCE/'semantic_e8/outputs/model.json').read_bytes()).hexdigest()}}}
    path=output/f'joe-{name}.json'; path.write_text(json.dumps(result,ensure_ascii=False,separators=(',',':'))+'\n'); print(name,path.stat().st_size)
manifest={p.name:{'sha256':hashlib.sha256(p.read_bytes()).hexdigest(),'bytes':p.stat().st_size} for p in sorted(output.glob('*.json'))}
(REPO/'docs/cdiss/fixtures/MANIFEST.json').write_text(json.dumps(manifest,indent=2)+'\n')
