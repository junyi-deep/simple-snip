"""Collect Cargo license declarations and available upstream license texts.
Run on the native target before packaging. No license exemption is inferred.
"""
import argparse,json,subprocess
from pathlib import Path
parser=argparse.ArgumentParser();parser.add_argument('--target',required=True);args=parser.parse_args()
if args.target not in ('aarch64-apple-darwin','x86_64-pc-windows-msvc'):parser.error('unsupported release target')
data=json.loads(subprocess.check_output(['cargo','metadata','--locked','--format-version','1','--filter-platform',args.target]))
reachable=set();nodes={n['id']:n for n in data['resolve']['nodes']}
def visit(package):
    if package in reachable:return
    reachable.add(package)
    for dependency in nodes[package]['deps']:visit(dependency['pkg'])
visit(data['resolve']['root'])
parts=['Simple Snip third-party notices\nTarget: '+args.target+'\nGenerated from Cargo.lock; inspect licenses before formal distribution.\n']
for package in sorted(data['packages'],key=lambda p:(p['name'],p['version'])):
    if package['id'] not in reachable:continue
    parts.append('\n'+'='*72+'\n'+package['name']+' '+package['version']+'\nLicense: '+str(package.get('license'))+'\nSource: '+str(package.get('repository'))+'\n')
    directory=Path(package['manifest_path']).parent
    candidates=set(directory.glob('LICENSE*'))|set(directory.glob('LICENCE*'))|set(directory.glob('COPYING*'))|set(directory.glob('NOTICE*'))
    if package.get('license_file'):candidates.add(directory/package['license_file'])
    for file in sorted(candidates):
        if file.is_file():parts.append('\n'+file.name+'\n'+file.read_text(encoding='utf-8',errors='replace'))
Path('dist').mkdir(exist_ok=True);Path('dist/THIRD_PARTY_NOTICES.txt').write_text('\n'.join(parts),encoding='utf-8')
print('Wrote notices for',len(reachable),'resolved packages')
