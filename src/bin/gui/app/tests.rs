
use diycpu::editor::SheetRef;
use diycpu::layout::PortKind;
use diycpu::project::PortRef;


use super::*;


/// Drive the app the way a user would: cursor moves and clicks at grid
/// positions, straight through the real gesture state machine.
pub struct Driver {
    pub app: App,
}

impl Driver {
    pub fn new() -> Self {
        let mut app = App::new(None);
        app.window_size = [1280.0, 720.0];
        app.pending_fit = false;
        // Park the origin comfortably inside the canvas.
        app.camera.offset = [400.0, 200.0];
        Driver { app }
    }

    pub fn move_to(&mut self, g: [f32; 2]) {
        let s = self.app.camera.grid_to_screen(g);
        self.app.mouse_moved(s);
    }

    pub fn click(&mut self, g: [f32; 2]) {
        self.move_to(g);
        self.app.left_pressed();
        self.app.left_released();
        // The shell recomputes hover every frame; mimic that.
        self.app.recompute_hover();
    }

    pub fn drag(&mut self, from: [f32; 2], to: [f32; 2]) {
        self.move_to(from);
        self.app.left_pressed();
        // Two moves so the drag threshold is crossed mid-gesture.
        self.move_to([(from[0] + to[0]) / 2.0, (from[1] + to[1]) / 2.0]);
        self.move_to(to);
        self.app.left_released();
        self.app.recompute_hover();
    }

    pub fn place(&mut self, type_name: &str, center: [f32; 2]) {
        self.app.palette_clicked(type_name);
        self.click(center);
    }

    pub fn type_text(&mut self, s: &str) {
        for c in s.chars() {
            self.app.handle_key(KeyInput::Char(&c.to_string()));
        }
    }
}

pub fn port_at(app: &App, comp: &str, port: &str) -> [f32; 2] {
    let (components, _) = app.editor.sheet_data();
    let c = components.iter().find(|c| c.id == comp).unwrap();
    diycpu::layout::port_pos(&app.editor.project.abstractions, c, port)
        .unwrap()
        .pos
}

pub fn center_of(app: &App, comp: &str) -> [f32; 2] {
    let (components, _) = app.editor.sheet_data();
    let c = components.iter().find(|c| c.id == comp).unwrap();
    let r = diycpu::layout::footprint(&app.editor.project.abstractions, c);
    [r.x + r.w / 2.0, r.y + r.h / 2.0]
}

#[test]
fn place_move_wire_flow() {
    let mut d = Driver::new();
    d.place("clk", [1.0, 1.0]);
    d.place("dff", [8.0, 1.0]);
    assert_eq!(d.app.editor.project.components.len(), 2);
    assert!(!d.app.status.is_error, "{}", d.app.status.text);

    // Wire clk.out → dff.clk by dragging port to port.
    let from = port_at(&d.app, "clk_1", "out");
    let to = port_at(&d.app, "dff_1", "clk");
    d.drag(from, to);
    assert_eq!(d.app.editor.project.wire_nets.len(), 1);
    assert_eq!(d.app.editor.project.wire_nets[0].sinks.len(), 1);

    // Move the dff; the wire follows because endpoints are symbolic.
    let before = d.app.editor.project.components[1].pos;
    let body = center_of(&d.app, "dff_1");
    d.drag(body, [body[0] + 5.0, body[1] + 2.0]);
    let dff = &d.app.editor.project.components[1];
    assert_eq!(dff.pos, [before[0] + 5.0, before[1] + 2.0]);
    assert_eq!(d.app.editor.project.wire_nets.len(), 1);
}

#[test]
fn click_click_wire_with_bend() {
    let mut d = Driver::new();
    d.place("nand", [0.0, 0.0]);
    d.place("nand", [10.0, 4.0]);

    let from = port_at(&d.app, "nand_1", "out");
    let to = port_at(&d.app, "nand_2", "a");
    d.click(from); // start wire
    assert!(matches!(d.app.gesture, Gesture::DrawingWire { .. }));
    d.click([6.0, 6.0]); // bend on empty canvas
    assert!(matches!(d.app.gesture, Gesture::DrawingWire { .. }));
    d.click(to); // finish on the input port
    assert!(matches!(d.app.gesture, Gesture::Idle));

    let net = &d.app.editor.project.wire_nets[0];
    assert_eq!(net.junctions.len(), 1);
    assert_eq!(net.junctions[0].pos, [6.0, 6.0]);
    assert_eq!(net.segments.len(), 2);
}

#[test]
fn wires_are_placed_objects_escape_keeps_them() {
    let mut d = Driver::new();
    d.place("nand", [0.0, 0.0]);

    // out → node → node, then stop: everything clicked stays placed.
    d.click(port_at(&d.app, "nand_1", "out"));
    d.click([6.0, 6.0]);
    d.click([8.0, 2.0]);
    assert!(matches!(d.app.gesture, Gesture::DrawingWire { .. }));
    d.app.handle_key(KeyInput::Escape);
    assert!(matches!(d.app.gesture, Gesture::Idle));
    let net = &d.app.editor.project.wire_nets[0];
    assert_eq!(net.junctions.len(), 2);
    assert_eq!(net.segments.len(), 2);
    assert_eq!(net.sinks.len(), 0);

    // Undo peels the wire back click by click.
    d.app.undo();
    assert_eq!(d.app.editor.project.wire_nets[0].junctions.len(), 1);
    d.app.undo();
    assert!(d.app.editor.project.wire_nets.is_empty());
}

#[test]
fn stopped_wire_resumes_from_its_node_from_either_side() {
    let mut d = Driver::new();
    d.place("nand", [0.0, 0.0]);
    d.place("nand", [10.0, 6.0]);

    // Output → node, stop drawing (the stub stays).
    d.click(port_at(&d.app, "nand_1", "out"));
    d.click([6.0, 6.0]);
    d.app.middle_or_right_pressed();
    assert!(matches!(d.app.gesture, Gesture::Idle));
    assert_eq!(d.app.editor.project.wire_nets[0].sinks.len(), 0);

    // Now start from the INPUT and finish on the stub's node.
    d.click(port_at(&d.app, "nand_2", "a"));
    assert!(matches!(d.app.gesture, Gesture::DrawingWire { .. }));
    d.click([6.0, 6.0]); // the node lives here
    assert!(matches!(d.app.gesture, Gesture::Idle));
    let net = &d.app.editor.project.wire_nets[0];
    assert_eq!(net.source, Some(PortRef {
        component_id: "nand_1".into(),
        port_name: "out".into(),
    }));
    assert_eq!(net.sinks.len(), 1);
}

#[test]
fn branch_from_a_mid_wire_node() {
    let mut d = Driver::new();
    d.place("nand", [0.0, 0.0]);
    d.place("nand", [10.0, 0.0]);
    d.place("nand", [10.0, 6.0]);

    // out → node → nand_2.a
    d.click(port_at(&d.app, "nand_1", "out"));
    d.click([6.0, 3.0]);
    d.click(port_at(&d.app, "nand_2", "a"));
    assert!(matches!(d.app.gesture, Gesture::Idle));

    // Click the node to branch, land on nand_3.a.
    d.click([6.0, 3.0]);
    assert!(matches!(d.app.gesture, Gesture::DrawingWire { .. }));
    d.click(port_at(&d.app, "nand_3", "a"));
    let net = &d.app.editor.project.wire_nets[0];
    assert_eq!(net.sinks.len(), 2);
    assert_eq!(net.junctions.len(), 1);
    assert_eq!(net.segments.len(), 3);
}

#[test]
fn drag_moves_a_node_and_delete_removes_it() {
    let mut d = Driver::new();
    d.place("nand", [0.0, 0.0]);
    d.click(port_at(&d.app, "nand_1", "out"));
    d.click([6.0, 3.0]);
    d.app.middle_or_right_pressed(); // stop drawing

    // Drag the node somewhere else.
    d.drag([6.0, 3.0], [9.0, 5.0]);
    assert_eq!(d.app.editor.project.wire_nets[0].junctions[0].pos, [9.0, 5.0]);

    // Hover it and press Delete: node and its wire go away.
    d.move_to([9.0, 5.0]);
    d.app.handle_key(KeyInput::Delete);
    assert!(d.app.editor.project.wire_nets.is_empty());
}

#[test]
fn invalid_drop_reports_and_leaves_document_alone() {
    let mut d = Driver::new();
    d.place("nand", [1.0, 1.0]);
    d.place("nand", [7.0, 1.0]);
    let before = d.app.editor.project.components[0].pos;
    // Try to drop nand_1 onto nand_2.
    d.drag(center_of(&d.app, "nand_1"), center_of(&d.app, "nand_2"));
    assert!(d.app.status.is_error);
    assert_eq!(d.app.editor.project.components[0].pos, before);
}

#[test]
fn band_select_and_delete() {
    let mut d = Driver::new();
    d.place("nand", [1.0, 1.0]); // top-left corner lands at (0, 0)
    d.place("nand", [7.0, 1.0]);
    d.place("nand", [1.0, 7.0]);
    d.drag([-2.0, -2.0], [11.0, 4.0]); // band over the top two
    assert_eq!(d.app.selection.components.len(), 2);
    d.app.handle_key(KeyInput::Delete);
    assert_eq!(d.app.editor.project.components.len(), 1);
    assert_eq!(d.app.editor.project.components[0].id, "nand_3");
}

#[test]
fn run_mode_locks_editing() {
    let mut d = Driver::new();
    d.place("dff", [0.0, 0.0]);
    d.app.action_run_toggle();
    assert!(d.app.is_running());

    d.app.palette_clicked("nand");
    assert!(matches!(d.app.gesture, Gesture::Idle));
    d.app.handle_key(KeyInput::Delete);
    assert_eq!(d.app.editor.project.components.len(), 1);

    d.app.handle_key(KeyInput::ArrowRight);
    // Esc exits run mode even with a live selection.
    d.click(center_of(&d.app, "dff_1"));
    d.app.handle_key(KeyInput::Escape);
    assert!(!d.app.is_running());
}

#[test]
fn autoplay_advances_and_manual_steps_pause() {
    let mut d = Driver::new();
    d.place("clk", [1.0, 1.0]);
    // P from edit mode: starts running AND playing.
    d.app.handle_key(KeyInput::Char("p"));
    assert!(d.app.is_running());
    assert!(d.app.playing);

    d.app.play_tick();
    d.app.play_tick();
    let Mode::Run(session) = &d.app.mode else { unreachable!() };
    assert_eq!(session.cursor, 2);

    // A manual step pauses playback.
    d.app.handle_key(KeyInput::ArrowLeft);
    assert!(!d.app.playing);
    let Mode::Run(session) = &d.app.mode else { unreachable!() };
    assert_eq!(session.cursor, 1);
    // Ticks do nothing while paused.
    d.app.play_tick();
    let Mode::Run(session) = &d.app.mode else { unreachable!() };
    assert_eq!(session.cursor, 1);

    // P resumes; stopping the run stops playback too.
    d.app.handle_key(KeyInput::Char("p"));
    assert!(d.app.playing);
    d.app.handle_key(KeyInput::Escape);
    assert!(!d.app.is_running());
    assert!(!d.app.playing);
}

#[test]
fn undo_mid_gesture_is_safe() {
    let mut d = Driver::new();
    d.place("nand", [0.0, 0.0]);
    d.place("nand", [8.0, 0.0]);
    d.click(port_at(&d.app, "nand_1", "out")); // start a wire
    d.app.undo(); // undo while drawing
    assert!(matches!(d.app.gesture, Gesture::Idle));
    assert_eq!(d.app.editor.project.components.len(), 1);
}

#[test]
fn build_abstraction_end_to_end_through_gestures() {
    let mut d = Driver::new();

    // Create the abstraction via the prompt.
    d.app.action_new_abstraction();
    d.type_text("inverter");
    d.app.handle_key(KeyInput::Enter);
    assert_eq!(d.app.editor.project.abstractions.len(), 1);
    assert!(matches!(d.app.editor.sheet, SheetRef::Abstraction(0)));

    // Add pins via prompts.
    d.app.action_add_pin(PortKind::Input);
    d.type_text("x");
    d.app.handle_key(KeyInput::Enter);
    d.app.action_add_pin(PortKind::Output);
    d.type_text("q");
    d.app.handle_key(KeyInput::Enter);

    // Place a nand and wire pin → gate → pin.
    d.place("nand", [1.5, 1.0]);
    let abstractions = &d.app.editor.project.abstractions;
    let def = &abstractions[0];
    let pins = diycpu::layout::interface_pins(abstractions, def);
    let in_pin = pins.iter().find(|p| p.name == "x").unwrap().pos;
    let out_pin = pins.iter().find(|p| p.name == "q").unwrap().pos;

    d.drag(in_pin, port_at(&d.app, "nand_1", "a"));
    d.drag(in_pin, port_at(&d.app, "nand_1", "b"));
    d.drag(port_at(&d.app, "nand_1", "out"), out_pin);

    let def = &d.app.editor.project.abstractions[0];
    assert_eq!(def.wire_nets.len(), 1);
    assert_eq!(def.wire_nets[0].sinks.len(), 2);
    assert_eq!(def.outputs.get("q"), Some(&"nand_1".to_string()));

    // Back to top (first Esc clears the selection, second navigates),
    // place an instance, feed it a constant 1, run.
    d.app.handle_key(KeyInput::Escape);
    d.app.handle_key(KeyInput::Escape);
    assert_eq!(d.app.editor.sheet, SheetRef::Top);
    d.place("dff", [1.0, 1.0]);
    d.app.double_clicked(); // cursor still over the dff → init = 1
    assert_eq!(d.app.editor.project.components[0].init, Some(true));
    d.place("inverter", [9.0, 1.0]);
    d.drag(
        port_at(&d.app, "dff_1", "out"),
        port_at(&d.app, "inverter_1", "x"),
    );

    d.app.action_run_toggle();
    assert!(d.app.is_running(), "compile failed: {}", d.app.status.text);
    let Mode::Run(session) = &d.app.mode else { unreachable!() };
    // NOT(1) = 0 on the instance output.
    assert_eq!(
        session.port_value_at(
            &[],
            false,
            &PortRef {
                component_id: "inverter_1".into(),
                port_name: "q".into()
            }
        ),
        Some(false)
    );
}
