"""Independent SciPy reference check of public Rust JS(base2)/TV outputs.
No network, provider or source-map writes. Scientific package version is recorded.
"""
import json, os, subprocess
from pathlib import Path
import numpy as np
import scipy
from scipy.spatial.distance import jensenshannon
root=Path(__file__).resolve().parents[2]
rng=np.random.default_rng(20261006)
pairs=[[[1,0],[0,1]], [[1,0],[.5,.5]], [[.5,.5],[.5,.5]]]
for i in range(32):
    a=rng.random(17); b=rng.random(17)
    a[a<.2]=0; b[b<.2]=0
    pairs.append([(a/a.sum()).tolist(),(b/b.sum()).tolist()])
env={**os.environ,'CARGO_TARGET_DIR':'/Users/paulcooper/Documents/Codex/2026-10-06/tak/work/bomb-code-primary/target'}
run=subprocess.run(['cargo','run','--quiet','-p','grok_cdiss','--example','js_parity'],input=json.dumps(pairs),text=True,cwd=root,env=env,capture_output=True,check=True)
actual=json.loads(run.stdout); errors=[]
for pair,out in zip(pairs,actual):
    a,b=np.asarray(pair[0]),np.asarray(pair[1])
    expected_js=float(jensenshannon(a,b,base=2)); expected_tv=float(np.abs(a-b).sum()/2)
    errors.append({'jsAbsoluteError':abs(expected_js-out['jensenShannonDistance']),'tvAbsoluteError':abs(expected_tv-out['totalVariation'])})
result={'scope':'independent SciPy parity on explicit normalized sparse identity allocations; no linguistic accuracy claim',
        'scipyVersion':scipy.__version__,'numpyVersion':np.__version__,'caseCount':len(pairs),
        'maxJsAbsoluteError':max(e['jsAbsoluteError'] for e in errors),'maxTvAbsoluteError':max(e['tvAbsoluteError'] for e in errors)}
assert result['maxJsAbsoluteError']<1e-12 and result['maxTvAbsoluteError']<1e-12,result
print(json.dumps(result,indent=2))
