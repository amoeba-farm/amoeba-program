"""Verify this source export without compiling, signing or contacting a chain."""
import hashlib
import json
from pathlib import Path
import re
import tomllib

bundle = Path(__file__).resolve().parent
root = bundle.parents[1]
# Preserve the 2026-10-22 Oracle source while the current Oracle source advances.
import subprocess, tempfile, tarfile, io
snapshot = tempfile.TemporaryDirectory(prefix='amoeba-oracle-20261022-historical-source-')
archive = subprocess.check_output(['git', '-C', str(root), 'archive', '--format=tar', '482f01f7774205595b13c83608c2693465420706', '--', 'oracle', 'futures', 'governance'])
root = Path(snapshot.name).resolve()
with tarfile.open(fileobj=io.BytesIO(archive)) as members:
    for member in members:
        target = (root / member.name).resolve()
        assert target.is_relative_to(root) and not member.issym() and not member.islnk()
        if member.isfile():
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(members.extractfile(member).read())
inventory = json.loads((bundle / 'SOURCE_FILES.json').read_text(encoding='utf8'))
assert inventory['sourceCommit'] == 'da0e9143f44635dcbce7288ea6510ddf042de215'
records = inventory['files']
assert len(records) == inventory['exportedFileCount']
paths = {entry['path'] for entry in records}
for entry in records:
    file = root / entry['path']
    assert not file.is_symlink(), entry['path']
    file = file.resolve()
    assert file.is_relative_to(root), entry['path']
    data = file.read_bytes()
    assert len(data) == entry['bytes'], entry['path']
    assert hashlib.sha256(data).hexdigest() == entry['sha256'], entry['path']
    assert hashlib.sha1(b'blob ' + str(len(data)).encode() + b'\0' + data).hexdigest() == entry['gitBlobSha1'], entry['path']
    assert entry['gitMode'] in ('100644', '100755'), entry['path']
    assert not any(part in ('.localnet', 'target', 'node_modules', '.next') or
                   part.startswith('.env') or part.endswith('-keypair.json')
                   for part in file.relative_to(root).parts), entry['path']

def check_path(base, path, manifest=True):
    resolved = (base / path).resolve()
    assert resolved.is_relative_to(root), str(resolved)
    assert (resolved / 'Cargo.toml').is_file() if manifest else resolved.is_file(), str(resolved)

manifests = [root / name for name in paths if name.endswith('Cargo.toml')]
for manifest in manifests:
    document = tomllib.loads(manifest.read_text(encoding='utf8'))
    workspace = document.get('workspace', {})
    for member in workspace.get('members', []):
        assert '*' not in member, 'Review globbed workspace member: ' + member
        check_path(manifest.parent, member)
    def dependencies(node):
        if isinstance(node, dict):
            for name, value in node.items():
                if name.endswith('dependencies') and isinstance(value, dict):
                    for spec in value.values():
                        if isinstance(spec, dict) and 'path' in spec:
                            check_path(manifest.parent, spec['path'])
                dependencies(value)
    dependencies(document)
for name in paths:
    if name.endswith('.rs'):
        file = root / name
        for relative in re.findall(r'include_(?:str|bytes)!\s*\(\s*"([^"]+)"', file.read_text(encoding='utf8')):
            check_path(file.parent, relative, manifest=False)
print(json.dumps({'sourceCommit': inventory['sourceCommit'], 'filesVerified': len(records),
                  'cargoManifestsChecked': len(manifests), 'buildRun': False,
                  'deployedByteReproductionClaimed': False}))
