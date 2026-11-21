# writes Cargo.toml + src/main.rs for pqdump over the crates named on the command line (paths under engine/)
import sys, re, os
here = os.path.dirname(os.path.abspath(__file__))
crates = sys.argv[1:]
deps = []
names = []
for c in crates:
    n = re.search(r'name = "([^"]+)"', open(f'{here}/../../{c}/Cargo.toml').read()).group(1)
    names.append(n)
    deps.append(f'{n} = {{ path = "../../{c}" }}')
open(f'{here}/Cargo.toml', 'w').write(f'''[package]
name = "pqdump"
version.workspace = true
edition.workspace = true

[dependencies]
vintage = {{ workspace = true, features = ["engine"] }}
{chr(10).join(deps)}
''')
reg = ',\n        '.join(f'{n}::pequod_native' for n in names)
open(f'{here}/src/main.rs', 'w').write(f'''//! Dumps each plugin's parameters (with value texts) and panel, to compare with the old VST3s.
use vintage::native::{{info_json, panel_json, Descriptor}};
use vintage::plugin::Midi;
use std::io::{{BufRead, Read, Write}};

/// Same job format as the VST3 probe: param <index> <norm> / in <file|silence n> / note <at> <pitch> <vel> / off <at> <pitch> / reset / out <file>
fn render(d: &Descriptor, job: &str) {{
    let inst = (d.make)();
    let block = 256usize;
    unsafe {{ inst.prepare(48000.0, 2, block); }}
    let mut input: Vec<f32> = Vec::new();
    let mut notes: Vec<(usize, u8, f32, bool)> = Vec::new();
    for line in std::io::BufReader::new(std::fs::File::open(job).unwrap()).lines() {{
        let line = line.unwrap();
        let w: Vec<&str> = line.split_whitespace().collect();
        match w.first().copied() {{
            Some("param") => inst.set_param(w[1].parse().unwrap(), w[2].parse().unwrap()),
            Some("plain") => {{ let i: usize = w[1].parse().unwrap(); inst.set_param(i, inst.normalize(i, w[2].parse().unwrap())) }}
            Some("in") => {{
                if w[1] == "silence" {{ input = vec![0.0; 2 * w[2].parse::<usize>().unwrap()]; }}
                else {{
                    let mut b = Vec::new();
                    std::fs::File::open(w[1]).unwrap().read_to_end(&mut b).unwrap();
                    input = b.chunks(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
                }}
            }}
            Some("note") => notes.push((w[1].parse().unwrap(), w[2].parse().unwrap(), w[3].parse().unwrap(), true)),
            Some("off") => notes.push((w[1].parse().unwrap(), w[2].parse().unwrap(), 0.0, false)),
            Some("reset") => unsafe {{ inst.reset() }},
            Some("out") => {{
                let frames = input.len() / 2;
                let mut out = vec![0f32; frames * 2];
                let mut pos = 0;
                while pos < frames {{
                    let n = block.min(frames - pos);
                    let mut l: Vec<f32> = (0..n).map(|i| input[2 * (pos + i)]).collect();
                    let mut r: Vec<f32> = (0..n).map(|i| input[2 * (pos + i) + 1]).collect();
                    for &(at, pitch, vel, on) in &notes {{
                        if at >= pos && at < pos + n {{
                            let ev = if on {{ Midi::On {{ note: pitch, vel }} }} else {{ Midi::Off {{ note: pitch }} }};
                            unsafe {{ inst.midi((at - pos) as u32, ev) }};
                        }}
                    }}
                    {{
                        let mut chans: [&mut [f32]; 2] = [&mut l[..], &mut r[..]];
                        unsafe {{ inst.process(&mut chans, Some(120.0), Some(pos as f64 / 48000.0 * 2.0), true) }};
                    }}
                    for i in 0..n {{ out[2 * (pos + i)] = l[i]; out[2 * (pos + i) + 1] = r[i]; }}
                    pos += n;
                }}
                let mut f = std::fs::File::create(w[1]).unwrap();
                for v in &out {{ f.write_all(&v.to_le_bytes()).unwrap(); }}
                notes.clear();
                println!("wrote {{}}", w[1]);
            }}
            _ => {{}}
        }}
    }}
}}
fn main() {{
    let all: Vec<fn() -> Descriptor> = vec![
        {reg}
    ];
    let args: Vec<String> = std::env::args().collect();
    if args.len() > 3 && args[1] == "render" {{
        for f in &all {{
            let d = f();
            if d.name == args[2].as_str() {{ render(&d, &args[3]); return; }}
        }}
        eprintln!("no plugin {{}}", args[2]);
        std::process::exit(1);
    }}
    let want = std::env::args().nth(1);
    print!("[");
    let mut first = true;
    for f in all {{
        let d = f();
        if let Some(w) = &want {{ if d.name != w.as_str() {{ continue; }} }}
        let inst = (d.make)();
        let mut texts = Vec::new();
        for (i, p) in inst.params().iter().enumerate() {{
            let n = if p.steps > 0 && p.steps < 64 {{ p.steps }} else {{ 20 }};
            let t: Vec<String> = (0..=n).map(|k| format!("{{:?}}", inst.param_text(i, k as f32 / n as f32))).collect();
            texts.push(format!("[{{}}]", t.join(",")));
        }}
        if !first {{ print!(","); }}
        first = false;
        println!("{{{{\\"class\\":{{:?}},\\"info\\":{{}},\\"texts\\":[{{}}],\\"panel\\":{{}}}}}}",
            String::from_utf8_lossy(&d.class_id), info_json(&d), texts.join(","), panel_json(&(d.panel)()));
    }}
    println!("]");
}}
''')
