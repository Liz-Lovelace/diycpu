use diycpu::project::Project;

/// Maximum number of undo states to keep in memory.
const MAX_UNDO_HISTORY: usize = 200;

/// A bounded undo/redo stack that stores snapshots of the project.
///
/// Each time the user makes a change, a snapshot is pushed onto the stack.
/// Undo/redo navigates through the history, and any new push after an undo
/// discards the "future" states (standard undo semantics).
pub struct UndoStack {
    snapshots: Vec<Project>,
    current_position: usize,
}

impl UndoStack {
    /// Create a new undo stack with the given initial state.
    pub fn new(initial_state: &Project) -> Self {
        Self {
            snapshots: vec![initial_state.clone()],
            current_position: 0,
        }
    }

    /// Push a new state onto the stack, discarding any redo history.
    pub fn push(&mut self, new_state: &Project) {
        // Discard any states after the current position (redo history).
        self.snapshots.truncate(self.current_position + 1);
        self.snapshots.push(new_state.clone());
        self.current_position = self.snapshots.len() - 1;

        // Keep the stack bounded to avoid unbounded memory growth.
        if self.snapshots.len() > MAX_UNDO_HISTORY {
            self.snapshots.remove(0);
            self.current_position -= 1;
        }
    }

    /// Move back one step in history. Returns the previous state, or None if already at the start.
    pub fn undo(&mut self) -> Option<&Project> {
        if self.current_position > 0 {
            self.current_position -= 1;
            Some(&self.snapshots[self.current_position])
        } else {
            None
        }
    }

    /// Move forward one step in history. Returns the next state, or None if already at the end.
    pub fn redo(&mut self) -> Option<&Project> {
        if self.current_position + 1 < self.snapshots.len() {
            self.current_position += 1;
            Some(&self.snapshots[self.current_position])
        } else {
            None
        }
    }

    /// Returns true if there is at least one state to undo to.
    pub fn can_undo(&self) -> bool {
        self.current_position > 0
    }

    /// Returns true if there is at least one state to redo to.
    pub fn can_redo(&self) -> bool {
        self.current_position + 1 < self.snapshots.len()
    }
}
