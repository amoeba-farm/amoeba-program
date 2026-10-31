"""Verify this source export without compiling, signing or contacting a chain."""
import hashlib
import json
from pathlib import Path
import re
import tomllib

bundle = Path(__file__).resolve().parent
root = bundle.parents[1]
repository = root
# Preserve the installed Futures source and its original dependencies.
import subprocess, tempfile, tarfile, io
snapshot = tempfile.TemporaryDirectory(prefix='amoeba-private-credit-historical-source-')
archive = subprocess.check_output(['git', '-C', str(root), 'archive', '--format=tar', '059bd727fb61b68adf7d5ee3067ed5158674cbbc', '--', 'oracle', 'futures', 'governance'])
root = Path(snapshot.name).resolve()
with tarfile.open(fileobj=io.BytesIO(archive)) as members:
    for member in members:
        target = (root / member.name).resolve()
        assert target.is_relative_to(root) and not member.issym() and not member.islnk()
        if member.isfile():
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(members.extractfile(member).read())
inventory = json.loads((bundle / 'SOURCE_FILES.json').read_text(encoding='utf8'))
assert inventory['sourceCommit'] == 'bcf0601606617fda7e00a818e7663d031339858f'
records = inventory['files']
assert len(records) == inventory['exportedFileCount']
dependencies = json.loads((bundle / 'DEPENDENCY_INPUTS.json').read_text())
assert dependencies['sourceCommit'] == inventory['sourceCommit']
records += dependencies['files']
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

reproduction = json.loads((bundle / 'REPRODUCTION.json').read_text())
assert reproduction['sourceCommit'] == inventory['sourceCommit']
for filename, key in [('SOURCE_FILES.json', 'sourceInventorySha256'), ('DEPENDENCY_INPUTS.json', 'dependencyInputsSha256')]:
    assert hashlib.sha256((bundle / filename).read_bytes()).hexdigest() == reproduction[key]
assert reproduction['artifactBytes'] == 722520 and reproduction['elfFlags'] == 1
assert reproduction['devMarkersFound'] == []
assert not (repository / 'futures/ops').exists(), 'Private operator configuration must not be exported'
print('Release metadata and private-configuration exclusion verified')
