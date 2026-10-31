"""Verify this source export without compiling, signing or contacting a chain."""
import hashlib
import json
from pathlib import Path
import re
import tomllib

bundle = Path(__file__).resolve().parent
root = bundle.parents[1]
inventory = json.loads((bundle / 'SOURCE_FILES.json').read_text(encoding='utf8'))
assert inventory['sourceCommit'] == '179864284b5ca09e2ce0bfdb94a05884d2e6f1ee'
assert inventory['releaseCommit'] == '2af39bf9f69f783bacf929daf9b1baa103ade351'
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
        for relative in re.findall(r'include(?:_str|_bytes)?!\s*\(\s*"([^"]+)"', file.read_text(encoding='utf8')):
            check_path(file.parent, relative, manifest=False)
print(json.dumps({'sourceCommit': inventory['sourceCommit'], 'filesVerified': len(records),
                  'cargoManifestsChecked': len(manifests), 'buildRun': False,
                  'deployedByteReproductionClaimed': False}))

# The production artifact is the build without explicit features: `heads` only.
program = tomllib.loads((root / 'oracle/programs/ameba_oracle/Cargo.toml').read_text(encoding='utf8'))
assert program['features']['default'] == ['heads']
reproduction = json.loads((bundle / 'REPRODUCTION.json').read_text())
assert reproduction['sourceCommit'] == inventory['sourceCommit']
assert reproduction['releaseCommit'] == inventory['releaseCommit']
assert hashlib.sha256((bundle / 'SOURCE_FILES.json').read_bytes()).hexdigest() == reproduction['sourceInventorySha256']
assert reproduction['artifactBytes'] == 726200 and reproduction['elfFlags'] == 1
assert reproduction['devMarkersFound'] == [] and reproduction['buildStackErrorLines'] == []
print('Release metadata and production feature selection verified')
