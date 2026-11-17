"""Check source/package boundaries without needing any private operator files."""
from pathlib import Path
import re

root=Path(__file__).resolve().parents[1]
bad=[]
for file in root.rglob('*'):
    rel=file.relative_to(root)
    if any(p in {'target','.git','dist'} for p in rel.parts) or not file.is_file():continue
    if file.name in {'invite.json','player-settings.json'} or file.suffix.lower() in {'.pem','.key','.pfx','.dpapi','.exe','.pdb'}:
        bad.append(str(rel));continue
    text=file.read_text(encoding='utf-8',errors='replace')
    if re.search(r'-----BEGIN (?:RSA |EC |OPENSSH )?PRIVATE KEY-----',text):bad.append(str(rel))
    if re.search(r'github_pat_[A-Za-z0-9_]{25,}|ghp_[A-Za-z0-9]{25,}',text):bad.append(str(rel))
if bad:raise SystemExit('Private/binary artifact found: '+', '.join(bad))
print('PASS: no invitations, private certificates/keys, credential files, executables, debug symbols or GitHub tokens in source tree')
