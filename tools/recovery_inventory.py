from pathlib import Path
import subprocess,json,re
import argparse
parser=argparse.ArgumentParser(description='Reproduce recovery file and public-declaration inventory from Git history')
parser.add_argument('--core',type=Path,required=True)
args=parser.parse_args()
r=Path(__file__).resolve().parents[1];core=args.core.resolve()
shas=['61073aa1007870f573d35ebb5285dfe9ee5d98c1','da37dcb2082986e8cc1f34c2d9d9d15daacc5dae','e759fbe7ba4be9127d8a533b4c5a9f69d342cef1','9bbf2d4fbcbb260fab6aae15de17fc575ee09075','e4d108df64413d19521f968d940f94ddeab8900b','4eafba6893bfa351aafe3d5ff571042ffeaacf3d','f93cde6','4379e21','6f43efb']
def git(repo,*args):return subprocess.check_output(['git','-C',str(repo),*args],stderr=subprocess.DEVNULL).decode()
def src(repo,ref,path):
 try:return git(repo,'show',ref+':'+path)
 except subprocess.CalledProcessError:return ''
def api(text):
 # Record exact public declarations/signatures and re-exports with their source lines.
 out=[]
 for m in re.finditer(r'(?m)^[ \t]*pub (?!\()(?:(?:async |unsafe |const )?fn\s+\w+[\s\S]*?(?=\{|;)|(?:struct|enum|trait|type|const|static|mod)\s+[^\n{;]+|use\s+[\s\S]*?;)',text):
  out.append({'line':text[:m.start()].count('\n')+1,'declaration':' '.join(m.group(0).split())})
 for m in re.finditer(r'(?m)^[ \t]*pub (?!\()\w+\s*:\s*[^\n]+',text):
  out.append({'line':text[:m.start()].count('\n')+1,'declaration':' '.join(m.group(0).split()),'kind':'public_field'})
 # Trait members are implicitly public; preserve their exact source blocks.
 for m in re.finditer(r'(?m)^[ \t]*pub trait [^\n{]+\{',text):
  depth=1; i=m.end()
  while i<len(text) and depth:
   depth += (text[i]=='{')-(text[i]=='}');i+=1
  out.append({'line':text[:m.start()].count('\n')+1,'declaration':text[m.start():i].strip(),'kind':'public_trait_contract'})
 return out
records=[];lines=['# Recovery inventory','', 'Baseline: `6259e0e95b4db8a3da91d553c3b321b102d4aec9`; audited tip: `6f43efb0ad984cd17c3b37ba4ddf73b850fb5e37`.','', 'This inventory records the original five commits and four subsequently published Connections commits, plus every corresponding Core change. It records every changed file per commit, exact public Rust declaration signatures and re-exports before/after, and recovery dispositions. Public declaration records include declarations inside modules; module visibility is recorded in the same inventory. Git additions/deletions measure diff lines, not independent features.','', 'The author’s apparent intent was a curated native account/timeline replacement followed by provider expansion. There are no explicit revert commits in these nine commits. Zomato was replaced with OAuth history, not restored with its earlier search/handoff service.','', '## Per-commit file changes','']
for sha in shas:
 sha=git(r,'rev-parse',sha).strip()
 parent=git(r,'rev-parse',sha+'^').strip();message=git(r,'log','-1','--format=%s',sha).strip()
 lines += [f'### {sha[:7]} — {message}', '', '| File | Classification | Added / deleted | Recovery disposition |','| --- | --- | --- | --- |']
 stat={p:(a,b) for a,b,p in [x.split('\t') for x in git(r,'diff','--numstat','--no-renames',parent,sha).splitlines()]}
 for row in git(r,'diff','--name-status','--find-renames',parent,sha).splitlines():
  parts=row.split('\t');status=parts[0];before=parts[1];after=parts[-1];kind={'D':'deleted','A':'newly added','M':'rewritten'}.get(status,'moved' if status.startswith('R') else status)
  if sha.startswith('9bb') and status=='M':kind='moved/refactored'
  if before=='src/connected_apps/crypto.rs' and status.startswith('R'):kind='moved and rewritten'
  disposition='Retained with current behavior'
  if status=='D':disposition='Restored from baseline'
  if after in ['src/providers/playstation.rs','src/providers/zomato.rs','src/providers/mod.rs','src/lib.rs','Cargo.toml','README.md','schema/README.md']:disposition='Combined: old public surface plus new provider/account behavior'
  if after=='schema/connectors.sql':disposition='Restored platform snapshot; curated schema separated into accounts.sql'
  if after=='src/crypto.rs':disposition='Shared implementation retained; old connected_apps::crypto adapter restored'
  if after=='src/accounts.rs':disposition='Retained; public ownership arguments changed to RequestScope, assistant reads require agent grant; 30-minute PSN cadence retained; history-removal intermediate superseded by final history callback'
  if after=='.github/workflows/ci.yml':disposition='Both platform and account suites restored'
  prev=src(r,parent,before) if status!='A' else '';next=src(r,sha,after) if status!='D' else ''
  a,b=stat.get(after,stat.get(before,('0','0')))
  rec={'commit':sha,'parent':parent,'before_path':before,'after_path':after,'status':status,'classification':kind,'added_lines':a,'deleted_lines':b,'disposition':disposition}
  if after.endswith('.rs') or before.endswith('.rs'):
   rec['public_before']=api(prev);rec['public_after']=api(next)
   old={v['declaration'] for v in rec['public_before']};new={v['declaration'] for v in rec['public_after']}
   rec['public_removed_or_changed']=sorted(old-new);rec['public_added_or_changed']=sorted(new-old)
  records.append(rec);lines.append(f'| `{after}`'+(f' ← `{before}`' if before!=after else '')+f' | {kind} | {a} / {b} | {disposition} |')
 lines.append('')
lines += ['## Public API changes','', 'Complete exact signatures, re-exports and source locations for every changed Rust file are in [recovery-inventory.json](recovery-inventory.json). Changed signatures appear once as a removal and once as an addition. This is an exact source declaration inventory, not a claim that each declaration was reachable through every old module export.','', 'Recovery restores all former module paths and provider service exports. Curated APIs retain their names but require `&impl RequestScope`; `assistant_read` and `read_personal` additionally require the selected agent key. Raw context UUID entry points are crate-private. `PlayStationGame.last_played_at` remains optional to retain provider uncertainty, and old consumers handle absent values without inventing timestamps.','', '## Core correspondence','', 'Core baseline: parent of `e4a9b96`; recovery includes upstream through `658387e`. Native connection APIs, worker scheduling, WiZ, map tools, notification delivery, new OAuth callback presentation and personal ingestion are retained. The table below accounts for every file changed by the corresponding retirement commit.','', '| Core file | Recovery disposition |','| --- | --- |']
core_rows=[]
for row in git(core,'diff','--name-status','e4a9b96^','e4a9b96').splitlines():
 status,path=row.split('\t',1)
 disposition='Retained; not part of connector recovery'
 if path.startswith('defaults/skills/') or path in ['src/identity_contract.rs','src/skills.rs','src/defaults.rs','src/skill_format.rs','src/capability_grants.rs','src/approval_contract.rs','src/privacy/content.rs']:disposition='Shared Connections implementation; compatibility re-export, no duplicate ownership'
 elif status=='D':disposition='Restored; compatibility adjusted where necessary'
 elif path in ['src/agents/tools/library.rs','src/delegation/mod.rs','src/durable_tasks/runs.rs','src/workers/task_executor.rs','src/approvals/mod.rs','src/http/mod.rs']:disposition='Restored governed behavior alongside retained native APIs'
 elif path.startswith('migrations/20261004'):disposition='Historical migration preserved; additive recovery migrations undo retirement safely'
 elif any(k in path for k in ['connection','identity','config','execution','privacy','lib.rs','status','openapi','services/api/main']):disposition='Retained and reconciled with restored platform where affected'
 lines.append(f'| `{path}` | {disposition} |');rec={'path':path,'status':status,'disposition':disposition}
 if path.endswith('.rs'):
  rec['public_before']=api(src(core,'e4a9b96^',path));rec['public_after']=api(src(core,'e4a9b96',path))
 core_rows.append(rec)

core_commits=[]
for sha in git(core,'rev-list','--reverse','e4a9b96..origin/main').splitlines():
 parent=git(core,'rev-parse',sha+'^').strip(); message=git(core,'log','-1','--format=%s',sha).strip()
 lines += [f'### Core {sha[:7]} — {message}', '', '| File | Classification | Recovery disposition |', '| --- | --- | --- |']
 for row in git(core,'diff','--name-status','--no-renames',parent,sha).splitlines():
  status,path=row.split('\t',1); rec={'commit':sha,'parent':parent,'path':path,'status':status,'classification':{'D':'deleted','A':'newly added','M':'rewritten'}[status],'disposition':('Superseded: preserve labelled observed ranges; no deletion of history or synthetic evenly-spaced sessions' if sha.startswith(('7c840e8','658387e')) and path=='src/fresh_connections.rs' else 'Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable')}
  if path.endswith('.rs'): rec['public_before']=api(src(core,parent,path));rec['public_after']=api(src(core,sha,path))
  core_commits.append(rec); lines.append(f"| `{path}` | {rec['classification']} | {rec['disposition']} |")
 lines.append('')
lines+=['','## Data that the code restoration cannot recover','', 'Core’s retirement migration revoked grants and external connections, expired unused decisions, dropped credential/OAuth state, packages/installations, setup records, PlayStation credential records and compact tool metadata, then renamed six surviving platform tables. Deployments which have not yet run retirement use an additive pre-retirement safeguard to preserve these rows and their existing authority. Already-retired deployments cannot recover deleted records: forward recovery restores names and empty storage, rebuilds metadata from retained declarations, and preserves surviving IDs and revoked states. It does not revive grants, approvals or deleted credentials. Packages require republication/review and affected accounts require reconnecting.','', 'Curated ciphertext retains its original user/provider associated-data interpretation. Context ownership is enforced separately by validated scopes, composite foreign keys and context-specific queries. Ambiguous legacy accounts retain IDs and ciphertext but are excluded from all reads/scheduled sync until explicit reassociation.','', '## Compatibility and release gates','', 'Host compatibility blocker: https://github.com/vox-suite/vox-web/issues/27. Native host UIs must expose explicit grants via restored authenticated platform routes. Linking does not auto-grant the Personal Assistant. The account read tool will deny access until a grant exists. Any host relying on automatic access must update its grant journey before release. Real provider account authorization and production callback allowlisting remain deployment validation gates.']
lines += ['', '## Current restored declarations', '', 'The JSON also records the current declarations for every library Rust source, including restored modules and compatibility signatures. Provider source declarations are distinct from deployment-verified access.']
current_api={str(p.relative_to(r)):api(p.read_text()) for p in sorted((r/'src').rglob('*.rs'))}
(r/'docs/recovery-inventory.md').write_text('\n'.join(lines)+'\n');(r/'docs/recovery-inventory.json').write_text(json.dumps({'current_core_public_declarations':{str(p.relative_to(core)):api(p.read_text()) for p in sorted((core/'src').rglob('*.rs'))},'current_connections_public_declarations':current_api,'connections_commits':records,'core_retirement':core_rows,'core_followup_commits':core_commits},indent=2)+'\n')
