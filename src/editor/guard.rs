//! The combinational-loop guard: runs the real engine pipeline after a
//! wire edit so the editor can refuse loops the instant they would form.



use crate::project::{Project, ProjectComponent};

use super::*;


/// Refuse wire edits that feed a signal combinationally back into itself
/// (a loop with no flip-flop in it). This runs the real engine pipeline —
/// convert, expand, toposort — so it can never disagree with Run. The sheet
/// being edited is checked as its own probe instance, since an abstraction
/// may not be placed anywhere yet. Pipeline failures for OTHER reasons are
/// ignored: unfinished circuits are legal editing states, and Run reports
/// its own errors.
pub(crate) fn guard_no_combinational_loop(project: &Project, sheet: SheetRef) -> Result<(), EditError> {
    let probe;
    let checked: &Project = match sheet {
        SheetRef::Top => project,
        SheetRef::Abstraction(i) => {
            let Some(def) = project.abstractions.get(i) else {
                return Ok(());
            };
            probe = Project {
                name: "__probe".into(),
                steps: 1,
                abstractions: project.abstractions.clone(),
                components: vec![ProjectComponent {
                    bits: None,
                    id: "__probe_instance".into(),
                    component_type: def.name.clone(),
                    init: None,
                    pos: [0.0, 0.0],
                }],
                wire_nets: vec![],
                notes: vec![],
                images: vec![],
            };
            &probe
        }
    };
    let def = match crate::convert::project_to_circuit(checked) {
        Ok(def) => def,
        // A bundle that resolves through itself IS a combinational loop.
        Err(crate::convert::ConvertError::BundleLoop { .. }) => {
            return Err(EditError::CombinationalLoop)
        }
        Err(_) => return Ok(()),
    };
    let Ok(expanded) = crate::expander::expand(&def) else {
        return Ok(());
    };
    match crate::compiler::compile(&expanded.def) {
        Err(
            crate::compiler::CompileError::CyclicGraph
            | crate::compiler::CompileError::SelfReference { .. },
        ) => Err(EditError::CombinationalLoop),
        _ => Ok(()),
    }
}
