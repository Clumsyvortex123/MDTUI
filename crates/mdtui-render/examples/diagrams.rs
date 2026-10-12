//! Print text renderings of mermaid diagrams: `cargo run -p mdtui-render --example diagrams [file.mmd]`
use mdtui_render::diagram::{render, Palette};

fn main() {
    let srcs: Vec<String> = match std::env::args().nth(1) {
        Some(p) => vec![std::fs::read_to_string(p).expect("read")],
        None => SAMPLES.iter().map(|s| s.to_string()).collect(),
    };
    for src in srcs {
        match render(&src, &Palette::default()) {
            Ok(d) => {
                for l in &d.lines {
                    println!("{}", l.spans.iter().map(|s| s.content.as_ref()).collect::<String>());
                }
                if let Some(n) = d.narrow {
                    println!("--- narrow:");
                    for l in &n {
                        println!("{}", l.spans.iter().map(|s| s.content.as_ref()).collect::<String>());
                    }
                }
            }
            Err(e) => println!("error: {e}"),
        }
        println!("==========");
    }
}

const SAMPLES: &[&str] = &[
    "flowchart LR\n    A[Markdown] --> B(comrak AST)\n    B --> C{IR}\n    C --> D[Layout]\n    D --> E[ratatui]",
    "flowchart TD\n    A[Start] --> B{Is it working?}\n    B -->|Yes| C[Ship it]\n    B -->|No| D[Debug]\n    D --> B\n    C --> E((Done))",
    "graph TD\n  A[Root] --> B[Left Node]\n  A --> C[Right Node]\n  B --> D[Final]\n  C --> D\n  A --> D",
    "sequenceDiagram\n    autonumber\n    participant U as User\n    participant M as mdtui\n    participant F as File\n    U->>M: open README.md\n    M->>F: read\n    F-->>M: markdown\n    M-->>U: rendered preview\n    Note over U,M: live reload\n    loop every save\n        F->>M: change event\n        M->>M: re-render\n    end",
    "stateDiagram-v2\n    [*] --> Still\n    Still --> [*]\n    Still --> Moving : push\n    Moving --> Still : stop\n    Moving --> Crash\n    Crash --> [*]",
    "pie title Pets adopted\n    \"Dogs\" : 386\n    \"Cats\" : 85\n    \"Rats\" : 15",
];
