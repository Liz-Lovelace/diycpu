use std::collections::HashMap;

use crate::compiler::CompiledCircuit;

/// Format simulation results as a human-readable text table.
///
/// Each row is one simulation step. Each column is a named component.
/// Values are shown as 0 or 1.
pub fn format_table(circuit: &CompiledCircuit, results: &[HashMap<String, bool>]) -> String {
    let component_names = &circuit.component_names;
    let step_column_width = results.len().to_string().len().max(4);
    let column_widths: Vec<usize> = component_names
        .iter()
        .map(|name| name.len().max(1))
        .collect();

    let mut output = String::new();

    // Header row with component names.
    output.push_str(&format!("{:>width$}", "Step", width = step_column_width));
    for (column_index, name) in component_names.iter().enumerate() {
        output.push_str(&format!(
            " | {:>width$}",
            name,
            width = column_widths[column_index]
        ));
    }
    output.push('\n');

    // Separator line.
    output.push_str(&"-".repeat(step_column_width));
    for column_width in &column_widths {
        output.push_str(&format!("-+-{}", "-".repeat(*column_width)));
    }
    output.push('\n');

    // Data rows, one per simulation step.
    for (step_number, step_state) in results.iter().enumerate() {
        output.push_str(&format!(
            "{:>width$}",
            step_number,
            width = step_column_width
        ));
        for (column_index, name) in component_names.iter().enumerate() {
            let value = if step_state[name] { 1 } else { 0 };
            output.push_str(&format!(
                " | {:>width$}",
                value,
                width = column_widths[column_index]
            ));
        }
        output.push('\n');
    }

    output
}
