"""Generate the pequod_engine crate: every plugin crate in the workspace, registered for the native engine."""
import os, re, glob
ROOT = os.path.dirname(os.path.abspath(__file__))
ws = open(f'{ROOT}/Cargo.toml').read()
members = re.findall(r'"([a-z0-9_]+)/\*"', ws)
crates = []
for m in members:
    for d in sorted(glob.glob(f'{ROOT}/{m}/*/Cargo.toml')):
        name = re.search(r'^name = "([^"]+)"', open(d).read(), re.M).group(1)
        lib = os.path.join(os.path.dirname(d), 'src/lib.rs')
        if 'vintage_plugin!' not in open(lib).read():
            continue
        crates.append((name, os.path.relpath(os.path.dirname(d), f'{ROOT}/pequod_engine')))
deps = '\n'.join(f'{n} = {{ path = "{p}" }}' for n, p in crates)
open(f'{ROOT}/pequod_engine/Cargo.toml', 'w').write(f'''[package]
name = "pequod_engine"
version.workspace = true
edition.workspace = true
license.workspace = true

[lib]
crate-type = ["staticlib", "rlib"]

[dependencies]
vintage = {{ workspace = true, features = ["engine"] }}
{deps}
''')
reg = ',\n        '.join(f'{n}::pequod_native' for n, _ in crates)
src = open(f'{ROOT}/pequod_engine/src/lib.in.rs').read().replace('/*REGISTRY*/', reg)
open(f'{ROOT}/pequod_engine/src/lib.rs', 'w').write(src)
print(len(crates), 'plugins')
