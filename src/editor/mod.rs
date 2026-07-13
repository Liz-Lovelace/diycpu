//! The editing layer: owns the `Project` document and applies validated,
//! atomic, undoable operations to it.
//!
//! Every mutation goes through [`Editor::mutate`], which snapshots the
//! document first and rolls back completely if the operation reports an
//! error — an operation can therefore never leave the document half-mutated.
//! One successful operation = exactly one undo step.
//!
//! Operations work on a *sheet*: either the top-level circuit or the inside
//! of one abstraction. Both have the same shape (components + wire nets),
//! so the same operations serve both.
//!
//! The map:
//! - this file — the `Editor` itself: document + sheet + undo machinery
//! - `sheets` — uniform sheet access, `PortResolver`, id generation
//! - `items` — components, notes, images on a sheet
//! - `wires` — the wire graph (nets, nodes, segments, sync/merge/split)
//! - `pins` — abstraction definitions and their interfaces
//! - `bundles` — lane counts for bundlers and bundle pins
//! - `clipboard` — copyable sheet fragments
//! - `guard` — the combinational-loop refusal
//! - `normalize` — the repair pass for loaded/hand-edited files

use std::fmt;

use crate::project::{
    Project, ProjectComponent, WireNet,
};


mod bundles;
mod clipboard;
mod guard;
mod items;
mod normalize;
mod pins;
mod sheets;
mod wires;

pub use clipboard::{Fragment, PastedIds};
pub use normalize::normalize;
pub use sheets::{abstraction_uses, sheet_images, sheet_notes, sheet_of};
pub use wires::{WireAttach, WirePoint};

// The op modules share their internals with each other through these
// crate-visible re-exports.
pub(crate) use guard::*;
pub(crate) use sheets::{
    generate_id, generate_image_id, generate_junction_id, generate_net_id,
    generate_note_id, sheet_images_mut, sheet_mut, sheet_notes_mut, PortResolver,
};
pub(crate) use wires::{
    drop_orphan_junctions, endpoint_is_port, gc_nets,
    merge_net, port_endpoint, repair_nets, split_all_nets,
    sync_net_logic,
};

#[cfg(test)]
mod bundle_tests;
#[cfg(test)]
mod image_tests;
#[cfg(test)]
mod passthrough_and_loop_tests;
#[cfg(test)]
mod tests;

/// Which circuit sheet operations apply to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SheetRef {
    Top,
    /// Index into `project.abstractions`.
    Abstraction(usize),
}

#[derive(Debug, Clone, PartialEq)]
pub enum EditError {
    UnknownComponentType(String),
    UnknownComponent(String),
    UnknownPort(String, String),
    Overlap,
    RecursiveAbstraction(String),
    NotAnInputAndOutput,
    NotADff(String),
    BadSheet,
    NothingToUndo,
    NothingToRedo,
    InvalidName(String),
    NameTaken(String),
    NotAnAbstractionSheet,
    /// The wire would feed a signal combinationally back into itself.
    CombinationalLoop,
    /// Bundle wires and signal wires can never join.
    MixedWireKinds,
    /// Two bundle ports of different lane counts on one net.
    BundleWidthMismatch(u32, u32),
    /// Lane counts are 2..=32 (a bundle of one is just a wire).
    BadBundleWidth,
    NotABundle(String),
    UnknownJunction(String),
    TwoSources(String),
    InputDoublyWired(String),
    /// The requested wire already exists / connects a point to itself —
    /// harmless, callers usually ignore it silently.
    NoopWire,
    OutputPinNeedsSource,
    AbstractionInUse(usize),
    UnknownNote(String),
    UnknownImage(String),
    UnknownNet(String),
}

impl fmt::Display for EditError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownComponentType(t) => write!(f, "Unknown component type \"{t}\""),
            Self::UnknownComponent(id) => write!(f, "No component \"{id}\""),
            Self::UnknownPort(c, p) => write!(f, "Component \"{c}\" has no port \"{p}\""),
            Self::Overlap => write!(f, "Components can't overlap"),
            Self::RecursiveAbstraction(name) => {
                write!(f, "Placing \"{name}\" here would nest it inside itself")
            }
            Self::NotAnInputAndOutput => {
                write!(f, "A wire connects an output to an input")
            }
            Self::NotADff(id) => write!(f, "\"{id}\" is not a flip-flop"),
            Self::BadSheet => write!(f, "This sheet no longer exists"),
            Self::NothingToUndo => write!(f, "Nothing to undo"),
            Self::NothingToRedo => write!(f, "Nothing to redo"),
            Self::InvalidName(n) => {
                write!(f, "\"{n}\" — names use letters, digits, _ and -")
            }
            Self::NameTaken(n) => write!(f, "The name \"{n}\" is already used"),
            Self::NotAnAbstractionSheet => {
                write!(f, "Pins can only be edited inside an abstraction")
            }
            Self::CombinationalLoop => {
                write!(
                    f,
                    "That would loop a signal straight back into itself — put a flip-flop in the loop"
                )
            }
            Self::MixedWireKinds => {
                write!(f, "Bundles and single wires don't mix — unbundle first")
            }
            Self::BundleWidthMismatch(a, b) => {
                write!(f, "These bundles are different sizes ({a} vs {b} lines)")
            }
            Self::BadBundleWidth => {
                write!(f, "Bundles carry 2 to 32 lines (a bundle of one is just a wire)")
            }
            Self::NotABundle(id) => write!(f, "\"{id}\" is not a bundler"),
            Self::UnknownJunction(id) => write!(f, "That wire node ({id}) no longer exists"),
            Self::TwoSources(existing) => {
                write!(f, "This wire is already driven by {existing}")
            }
            Self::InputDoublyWired(port) => {
                write!(f, "{port} is already connected to this wire")
            }
            Self::NoopWire => write!(f, "Those points are already connected"),
            Self::OutputPinNeedsSource => {
                write!(f, "This wire has no output driving it yet — connect one first")
            }
            Self::AbstractionInUse(n) => write!(
                f,
                "This abstraction is placed {n} time(s) — delete those instances first"
            ),
            Self::UnknownNote(id) => write!(f, "That note ({id}) no longer exists"),
            Self::UnknownImage(id) => write!(f, "That image ({id}) no longer exists"),
            Self::UnknownNet(id) => write!(f, "That wire ({id}) no longer exists"),
        }
    }
}

/// Names for abstractions and pins: they become engine identifiers, so no
/// dots, colons or whitespace.
const UNDO_LIMIT: usize = 200;

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

pub struct Editor {
    pub project: Project,
    /// The sheet currently open in the workbench.
    pub sheet: SheetRef,
    undo_stack: Vec<(Project, SheetRef)>,
    redo_stack: Vec<(Project, SheetRef)>,
    pub dirty: bool,
}

impl Editor {
    pub fn new(project: Project) -> Self {
        Editor {
            project,
            sheet: SheetRef::Top,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            dirty: false,
        }
    }

    pub fn empty() -> Self {
        Self::new(Project {
            name: "untitled".into(),
            steps: 16,
            abstractions: vec![],
            components: vec![],
            wire_nets: vec![],
            notes: vec![],
            images: vec![],
        })
    }

    /// The components and wire nets of the current sheet.
    pub fn sheet_data(&self) -> (&[ProjectComponent], &[WireNet]) {
        sheet_of(&self.project, self.sheet)
            .expect("current sheet is always kept valid")
    }

    /// The annotations on the current sheet.
    pub fn sheet_images(&self) -> &[crate::project::MediaImage] {
        sheet_images(&self.project, self.sheet).unwrap_or(&[])
    }

    pub fn sheet_notes(&self) -> &[crate::project::Note] {
        sheet_notes(&self.project, self.sheet)
            .expect("current sheet is always kept valid")
    }

    pub fn open_sheet(&mut self, sheet: SheetRef) -> Result<(), EditError> {
        if sheet_of(&self.project, sheet).is_none() {
            return Err(EditError::BadSheet);
        }
        self.sheet = sheet;
        Ok(())
    }

    /// Run one atomic operation. On success it becomes one undo step; on
    /// error the document is exactly as before the call.
    fn mutate<R>(
        &mut self,
        op: impl FnOnce(&mut Project, SheetRef) -> Result<R, EditError>,
    ) -> Result<R, EditError> {
        let before = self.project.clone();
        match op(&mut self.project, self.sheet) {
            Ok(result) => {
                self.undo_stack.push((before, self.sheet));
                if self.undo_stack.len() > UNDO_LIMIT {
                    self.undo_stack.remove(0);
                }
                self.redo_stack.clear();
                self.dirty = true;
                Ok(result)
            }
            Err(e) => {
                self.project = before;
                Err(e)
            }
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    pub fn undo(&mut self) -> Result<(), EditError> {
        let (project, sheet) = self.undo_stack.pop().ok_or(EditError::NothingToUndo)?;
        self.redo_stack
            .push((std::mem::replace(&mut self.project, project), self.sheet));
        self.sheet = sheet;
        self.dirty = true;
        Ok(())
    }

    pub fn redo(&mut self) -> Result<(), EditError> {
        let (project, sheet) = self.redo_stack.pop().ok_or(EditError::NothingToRedo)?;
        self.undo_stack
            .push((std::mem::replace(&mut self.project, project), self.sheet));
        self.sheet = sheet;
        self.dirty = true;
        Ok(())
    }
}
