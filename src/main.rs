use std::process;

use diycpu::compiler::compile;
use diycpu::expander::expand;
use diycpu::simulator::{format_table, simulate};
use diycpu::types::CircuitDef;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 2 {
        eprintln!("Usage: diycpu <circuit.yaml>");
        process::exit(1);
    }

    let path = &args[1];
    let contents = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Error reading {path}: {e}");
            process::exit(1);
        }
    };

    let def: CircuitDef = match serde_yaml::from_str(&contents) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("Error parsing YAML: {e}");
            process::exit(1);
        }
    };

    let expanded = match expand(&def) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("Error expanding abstractions: {e}");
            process::exit(1);
        }
    };

    let circuit = match compile(&expanded.def) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Error compiling circuit: {e}");
            process::exit(1);
        }
    };

    let mut results = simulate(&circuit);

    // Add output aliases so abstraction outputs are accessible by dot-notation names.
    for step in &mut results {
        for (alias, target) in &expanded.aliases {
            if let Some(&val) = step.get(target) {
                step.insert(alias.clone(), val);
            }
        }
    }

    println!("=== {} ===", circuit.name);
    print!("{}", format_table(&circuit, &results));
}
