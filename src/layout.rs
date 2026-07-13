//! Pure geometry and port metadata for the GUI: the layer between the
//! document (`Project`) and anything visual or interactive.
//!
//! Everything here is in **grid units** (the renderer decides how many
//! pixels a grid cell is). Component positions snap to integer cells;
//! ports sit on component edges. Nothing in this module holds state and
//! nothing depends on a rendering backend.

use crate::project::{Junction, ProjectAbstraction, ProjectComponent, WireNet};

/// Grid-unit distance a wire travels straight out of a port before turning.
pub const PORT_STUB: f32 = 0.5;

/// Hit-test radius around a port center, in grid units.
pub const PORT_HIT_RADIUS: f32 = 0.4;

/// Hit-test radius around a wire node (junction), in grid units.
pub const JUNCTION_HIT_RADIUS: f32 = 0.35;

/// Hit-test distance from a wire centerline, in grid units.
pub const WIRE_HIT_RADIUS: f32 = 0.25;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortKind {
    Input,
    Output,
}

#[derive(Debug, Clone)]
pub struct PortInfo {
    pub name: String,
    pub kind: PortKind,
}

/// Axis-aligned rectangle in grid units.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn intersects(&self, other: &Rect) -> bool {
        self.x < other.x + other.w
            && other.x < self.x + self.w
            && self.y < other.y + other.h
            && other.y < self.y + self.h
    }

    pub fn contains(&self, p: [f32; 2]) -> bool {
        p[0] >= self.x && p[0] <= self.x + self.w && p[1] >= self.y && p[1] <= self.y + self.h
    }
}

/// The port list of a component type, in display order (top to bottom).
/// Returns None for unknown types.
pub fn type_ports(
    abstractions: &[ProjectAbstraction],
    component_type: &str,
) -> Option<Vec<PortInfo>> {
    let port = |name: &str, kind| PortInfo {
        name: name.to_string(),
        kind,
    };
    match component_type {
        "clk" => Some(vec![port("out", PortKind::Output)]),
        "nand" => Some(vec![
            port("a", PortKind::Input),
            port("b", PortKind::Input),
            port("out", PortKind::Output),
        ]),
        "dff" => Some(vec![
            port("d", PortKind::Input),
            port("clk", PortKind::Input),
            port("out", PortKind::Output),
        ]),
        name => {
            let def = abstractions.iter().find(|a| a.name == name)?;
            let mut ports: Vec<PortInfo> = def
                .inputs
                .iter()
                .map(|i| port(i, PortKind::Input))
                .collect();
            // HashMap order is unstable; sort output names for determinism.
            let mut outs: Vec<&String> = def.outputs.keys().collect();
            outs.sort();
            ports.extend(outs.into_iter().map(|o| port(o, PortKind::Output)));
            Some(ports)
        }
    }
}

/// Footprint size in grid units for a component type. None for unknown types.
pub fn type_size(abstractions: &[ProjectAbstraction], component_type: &str) -> Option<[f32; 2]> {
    match component_type {
        "clk" => Some([2.0, 2.0]),
        "nand" | "dff" => Some([3.0, 2.0]),
        name => {
            let def = abstractions.iter().find(|a| a.name == name)?;
            Some([def.width, def.height])
        }
    }
}

/// Default footprint for a newly created abstraction, from its interface.
pub fn default_abstraction_size(name: &str, n_inputs: usize, n_outputs: usize) -> [f32; 2] {
    let w = (3.0f32).max((name.len() as f32 * 0.45).ceil() + 1.0);
    let h = (2.0f32).max(n_inputs.max(n_outputs) as f32);
    [w, h]
}

/// Footprint rectangle of a placed component.
pub fn footprint(abstractions: &[ProjectAbstraction], comp: &ProjectComponent) -> Rect {
    let [w, h] = type_size(abstractions, &comp.component_type).unwrap_or([3.0, 2.0]);
    Rect {
        x: comp.pos[0],
        y: comp.pos[1],
        w,
        h,
    }
}

/// Which side of the component a port sits on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}

impl Side {
    /// Unit x-direction a wire leaves/enters this side (outward normal).
    pub fn outward_x(self) -> f32 {
        match self {
            Side::Left => -1.0,
            Side::Right => 1.0,
        }
    }
}

#[derive(Debug, Clone)]
pub struct PortGeom {
    pub info: PortInfo,
    /// Center of the port circle, in grid units (on the component edge).
    pub pos: [f32; 2],
    pub side: Side,
}

/// Geometry of every port on a placed component: inputs spaced evenly down
/// the left edge, outputs down the right edge.
pub fn component_ports(
    abstractions: &[ProjectAbstraction],
    comp: &ProjectComponent,
) -> Vec<PortGeom> {
    let Some(ports) = type_ports(abstractions, &comp.component_type) else {
        return Vec::new();
    };
    let rect = footprint(abstractions, comp);

    let mut inputs: Vec<&PortInfo> =
        ports.iter().filter(|p| p.kind == PortKind::Input).collect();
    let mut outputs: Vec<&PortInfo> =
        ports.iter().filter(|p| p.kind == PortKind::Output).collect();

    // Abstraction instances order their ports the way the pins are laid
    // out inside the sheet (top to bottom) — arranging the pins inside is
    // how you arrange the ports outside.
    if let Some(def) = abstractions.iter().find(|a| a.name == comp.component_type) {
        let pins = interface_pins(abstractions, def);
        let key = |name: &str| {
            pins.iter()
                .find(|p| p.name == name)
                .map(|p| p.pos[1])
                .unwrap_or(0.0)
        };
        inputs.sort_by(|a, b| {
            key(&a.name)
                .total_cmp(&key(&b.name))
                .then_with(|| a.name.cmp(&b.name))
        });
        outputs.sort_by(|a, b| {
            key(&a.name)
                .total_cmp(&key(&b.name))
                .then_with(|| a.name.cmp(&b.name))
        });
    }

    let mut result = Vec::with_capacity(ports.len());
    for (list, side) in [(inputs, Side::Left), (outputs, Side::Right)] {
        let n = list.len();
        for (i, info) in list.into_iter().enumerate() {
            let x = match side {
                Side::Left => rect.x,
                Side::Right => rect.x + rect.w,
            };
            let y = rect.y + rect.h * (i as f32 + 0.5) / n as f32;
            result.push(PortGeom {
                info: info.clone(),
                pos: [x, y],
                side,
            });
        }
    }
    result
}

/// Position of one named port on a placed component.
pub fn port_pos(
    abstractions: &[ProjectAbstraction],
    comp: &ProjectComponent,
    port_name: &str,
) -> Option<PortGeom> {
    component_ports(abstractions, comp)
        .into_iter()
        .find(|p| p.info.name == port_name)
}

// ── Wire routing ─────────────────────────────────────────────────────────────
//
// There is exactly one routing algorithm. Committed wires, the live drag
// preview, and wire hit-testing all call `route`, so what you see while
// drawing is always what you get.

/// Direction a wire must travel when leaving an endpoint. `None` for free
/// endpoints (junctions, the cursor during a drag).
pub type ExitDir = Option<Side>;

/// Grid clearance a backward route keeps above the endpoints it detours over.
pub const DETOUR: f32 = 1.5;

/// Route an orthogonal polyline from `a` to `b`.
///
/// `a_dir`/`b_dir` are the sides the endpoints exit toward (outputs exit
/// Right, inputs are approached from their Left side, i.e. exit Left).
pub fn route(a: [f32; 2], a_dir: ExitDir, b: [f32; 2], b_dir: ExitDir) -> Vec<[f32; 2]> {
    // Stub points: step straight out of each port before any turns.
    let sa = stub(a, a_dir);
    let sb = stub(b, b_dir);

    let mut pts = vec![a];
    if sa != a {
        pts.push(sa);
    }
    connect_orthogonal(&mut pts, sa, sb, a_dir, b_dir);
    if sb != b {
        pts.push(sb);
    }
    pts.push(b);
    dedup_consecutive(&mut pts);
    pts
}

fn stub(p: [f32; 2], dir: ExitDir) -> [f32; 2] {
    match dir {
        Some(side) => [p[0] + side.outward_x() * PORT_STUB, p[1]],
        None => p,
    }
}

/// Append the elbow points connecting stub point `sa` to stub point `sb`
/// (exclusive of both, which the caller pushes).
fn connect_orthogonal(
    pts: &mut Vec<[f32; 2]>,
    sa: [f32; 2],
    sb: [f32; 2],
    a_dir: ExitDir,
    b_dir: ExitDir,
) {
    let dx = sb[0] - sa[0];

    // Would horizontal travel in `x_dir` leave sa without doubling back
    // over a's stub, and arrive at sb without doubling back over b's stub?
    let leaves_a_cleanly =
        |x_dir: f32| a_dir.map_or(true, |side| x_dir * side.outward_x() >= 0.0);
    let arrives_b_cleanly =
        |x_dir: f32| b_dir.map_or(true, |side| x_dir * side.outward_x() <= 0.0);
    let horizontal_ok = dx != 0.0
        && leaves_a_cleanly(dx.signum())
        && arrives_b_cleanly(dx.signum());

    if sa[0] == sb[0] {
        // Vertical straight run between the stubs — always clean, the stubs
        // themselves provide the horizontal entry/exit.
        return;
    }
    if sa[1] == sb[1] && horizontal_ok {
        return; // straight horizontal run
    }

    if horizontal_ok {
        // H-V-H through the midpoint: ┌─┐ shape.
        let mid_x = (sa[0] + sb[0]) / 2.0;
        pts.push([mid_x, sa[1]]);
        pts.push([mid_x, sb[1]]);
    } else {
        // The target is behind one of the exits: detour around, above the
        // higher of the two endpoints, so the wire arcs over components
        // instead of cutting through the space between them.
        let detour_y = sa[1].min(sb[1]) - DETOUR;
        pts.push([sa[0], detour_y]);
        pts.push([sb[0], detour_y]);
    }
}

fn dedup_consecutive(pts: &mut Vec<[f32; 2]>) {
    pts.dedup_by(|a, b| (a[0] - b[0]).abs() < 1e-6 && (a[1] - b[1]).abs() < 1e-6);
}

// ── Abstraction interface pins ───────────────────────────────────────────────
//
// When editing the inside of an abstraction, its inputs and outputs appear
// as pins on rails beside the content. Pins have no stored position — their
// geometry is derived from the content bounds, so they can never go stale.
// In wire data, an input pin is the pseudo-port `INPUT:<name>` (which the
// backend format already uses); an output pin is `OUTPUT:<name>`, which is
// GUI-only: connecting to it writes the abstraction's `outputs` map.

/// GUI-only pseudo-component prefix for abstraction output pins.
pub const OUTPUT_PREFIX: &str = "OUTPUT:";

#[derive(Debug, Clone)]
pub struct PinGeom {
    pub name: String,
    /// User-facing direction: Input = an input pin of the abstraction.
    pub io: PortKind,
    pub pos: [f32; 2],
    /// Clickable name-plate rectangle next to the pin circle.
    pub plate: Rect,
}

impl PinGeom {
    /// How the pin behaves in wires: an input pin *drives* nets (acts as an
    /// output port); an output pin is *fed* by them.
    pub fn wire_kind(&self) -> PortKind {
        match self.io {
            PortKind::Input => PortKind::Output,
            PortKind::Output => PortKind::Input,
        }
    }

    /// The pseudo PortRef this pin participates in wires as.
    pub fn port_ref(&self) -> crate::project::PortRef {
        let prefix = match self.io {
            PortKind::Input => crate::convert::INPUT_PREFIX,
            PortKind::Output => OUTPUT_PREFIX,
        };
        crate::project::PortRef {
            component_id: format!("{prefix}{}", self.name),
            port_name: self.name.clone(),
        }
    }
}

/// Content bounding box of a set of components, in grid units.
pub fn components_bounds(
    abstractions: &[ProjectAbstraction],
    components: &[ProjectComponent],
) -> Option<Rect> {
    let mut bounds: Option<Rect> = None;
    for c in components {
        let r = footprint(abstractions, c);
        bounds = Some(match bounds {
            None => r,
            Some(b) => {
                let x = b.x.min(r.x);
                let y = b.y.min(r.y);
                let x2 = (b.x + b.w).max(r.x + r.w);
                let y2 = (b.y + b.h).max(r.y + r.h);
                Rect { x, y, w: x2 - x, h: y2 - y }
            }
        });
    }
    bounds
}

/// The rail position a pin defaults to before it has been dragged anywhere:
/// inputs stack left of the content, outputs to the right.
pub fn default_pin_position(
    abstractions: &[ProjectAbstraction],
    def: &ProjectAbstraction,
    io: PortKind,
    index: usize,
) -> [f32; 2] {
    let bounds = components_bounds(abstractions, &def.components).unwrap_or(Rect {
        x: 0.0,
        y: 0.0,
        w: 10.0,
        h: 4.0,
    });
    let x = match io {
        PortKind::Input => (bounds.x - 4.0).floor(),
        PortKind::Output => (bounds.x + bounds.w + 4.0).ceil(),
    };
    [x, bounds.y + 0.5 + index as f32 * 2.0]
}

/// The interface pins of an abstraction, at their stored positions (pins
/// are free-floating objects inside the sheet); un-dragged pins fall back
/// to the rails.
pub fn interface_pins(
    abstractions: &[ProjectAbstraction],
    def: &ProjectAbstraction,
) -> Vec<PinGeom> {
    let mut pins = Vec::new();
    for (i, name) in def.inputs.iter().enumerate() {
        let pos = def
            .pin_positions
            .get(name)
            .copied()
            .unwrap_or_else(|| default_pin_position(abstractions, def, PortKind::Input, i));
        pins.push(PinGeom {
            name: name.clone(),
            io: PortKind::Input,
            pos,
            plate: Rect {
                x: pos[0] - 2.6,
                y: pos[1] - 0.45,
                w: 2.2,
                h: 0.9,
            },
        });
    }
    let mut outs: Vec<&String> = def.outputs.keys().collect();
    outs.sort();
    for (i, name) in outs.into_iter().enumerate() {
        let pos = def
            .pin_positions
            .get(name)
            .copied()
            .unwrap_or_else(|| default_pin_position(abstractions, def, PortKind::Output, i));
        pins.push(PinGeom {
            name: name.clone(),
            io: PortKind::Output,
            pos,
            plate: Rect {
                x: pos[0] + 0.4,
                y: pos[1] - 0.45,
                w: 2.2,
                h: 0.9,
            },
        });
    }
    pins
}

/// Position + exit direction for any port reference on a sheet, including
/// the pseudo-ports of interface pins.
pub fn port_geom_any(
    abstractions: &[ProjectAbstraction],
    components: &[ProjectComponent],
    interface: Option<&ProjectAbstraction>,
    port: &crate::project::PortRef,
) -> Option<([f32; 2], Side)> {
    if let Some(name) = port.component_id.strip_prefix(crate::convert::INPUT_PREFIX) {
        let def = interface?;
        let pin = interface_pins(abstractions, def)
            .into_iter()
            .find(|p| p.name == name && p.io == PortKind::Input)?;
        return Some((pin.pos, Side::Right)); // input pins feed rightward
    }
    if let Some(name) = port.component_id.strip_prefix(OUTPUT_PREFIX) {
        let def = interface?;
        let pin = interface_pins(abstractions, def)
            .into_iter()
            .find(|p| p.name == name && p.io == PortKind::Output)?;
        return Some((pin.pos, Side::Left)); // output pins are fed from the left
    }
    let comp = components.iter().find(|c| c.id == port.component_id)?;
    let geom = port_pos(abstractions, comp, &port.port_name)?;
    Some((geom.pos, geom.side))
}

/// Parse an abstraction `outputs` map target ("comp" or "comp.port") into a
/// port reference on the sheet.
pub fn output_target_port(target: &str) -> crate::project::PortRef {
    match target.split_once('.') {
        Some((id, port)) => crate::project::PortRef {
            component_id: id.to_string(),
            port_name: port.to_string(),
        },
        None => crate::project::PortRef {
            component_id: target.to_string(),
            port_name: "out".to_string(),
        },
    }
}

// ── Nested-sheet transforms (infinite zoom) ──────────────────────────────────
//
// An abstraction instance displays its own sheet scaled into its footprint.
// `SheetMap` is that similarity transform (child grid units → parent grid
// units). Composing maps and camera views is how the whole "one infinitely
// zoomable canvas" illusion is built.

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SheetMap {
    /// Child units per parent unit.
    pub s: f32,
    /// Parent-space position of the child origin.
    pub o: [f32; 2],
}

impl SheetMap {
    pub fn apply(&self, p: [f32; 2]) -> [f32; 2] {
        [p[0] * self.s + self.o[0], p[1] * self.s + self.o[1]]
    }

    pub fn inverse(&self) -> SheetMap {
        SheetMap {
            s: 1.0 / self.s,
            o: [-self.o[0] / self.s, -self.o[1] / self.s],
        }
    }
}

/// Fraction of the instance rect the child content is inset to, leaving a
/// margin inside the body border.
pub const INNER_FIT: f32 = 0.92;

/// Everything on an abstraction's sheet, padded — the region an instance
/// shows when you look inside it.
pub fn abstraction_inner_bounds(
    abstractions: &[ProjectAbstraction],
    def: &ProjectAbstraction,
) -> Rect {
    let mut bounds = components_bounds(abstractions, &def.components);
    let mut extend = |r: Rect| {
        bounds = Some(match bounds {
            None => r,
            Some(b) => {
                let x = b.x.min(r.x);
                let y = b.y.min(r.y);
                let x2 = (b.x + b.w).max(r.x + r.w);
                let y2 = (b.y + b.h).max(r.y + r.h);
                Rect { x, y, w: x2 - x, h: y2 - y }
            }
        });
    };
    for note in &def.notes {
        extend(note_bounds(note));
    }
    for pin in interface_pins(abstractions, def) {
        extend(pin.plate);
        extend(Rect {
            x: pin.pos[0] - 0.5,
            y: pin.pos[1] - 0.5,
            w: 1.0,
            h: 1.0,
        });
    }
    let b = bounds.unwrap_or(Rect {
        x: 0.0,
        y: 0.0,
        w: 10.0,
        h: 6.0,
    });
    const PAD: f32 = 1.0;
    Rect {
        x: b.x - PAD,
        y: b.y - PAD,
        w: b.w + PAD * 2.0,
        h: b.h + PAD * 2.0,
    }
}

/// The transform that fits `def`'s sheet inside an instance footprint.
pub fn instance_inner_map(
    abstractions: &[ProjectAbstraction],
    def: &ProjectAbstraction,
    instance_rect: Rect,
) -> SheetMap {
    let b = abstraction_inner_bounds(abstractions, def);
    let s = (instance_rect.w / b.w)
        .min(instance_rect.h / b.h)
        .max(1e-6)
        * INNER_FIT;
    SheetMap {
        s,
        o: [
            instance_rect.x + (instance_rect.w - b.w * s) / 2.0 - b.x * s,
            instance_rect.y + (instance_rect.h - b.h * s) / 2.0 - b.y * s,
        ],
    }
}

// ── Notes ────────────────────────────────────────────────────────────────────
//
// Notes wrap with a deterministic character-count algorithm so the hit
// rectangle and the rendered text always agree (no font metrics needed).

pub const NOTE_WRAP_CHARS: usize = 26;
pub const NOTE_LINE_HEIGHT: f32 = 0.55;
pub const NOTE_CHAR_WIDTH: f32 = 0.24;
pub const NOTE_PADDING: f32 = 0.3;

/// Greedy word wrap at [`NOTE_WRAP_CHARS`]; over-long words are hard-broken.
pub fn wrap_note_text(text: &str) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        let mut word = word.to_string();
        loop {
            let need = if current.is_empty() {
                word.chars().count()
            } else {
                current.chars().count() + 1 + word.chars().count()
            };
            if need <= NOTE_WRAP_CHARS {
                if !current.is_empty() {
                    current.push(' ');
                }
                current.push_str(&word);
                break;
            }
            if current.is_empty() {
                // A single word longer than the line: hard-break it.
                let head: String = word.chars().take(NOTE_WRAP_CHARS).collect();
                let tail: String = word.chars().skip(NOTE_WRAP_CHARS).collect();
                lines.push(head);
                word = tail;
                if word.is_empty() {
                    break;
                }
            } else {
                lines.push(std::mem::take(&mut current));
            }
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

/// The clickable rectangle of a note, in grid units.
pub fn note_bounds(note: &crate::project::Note) -> Rect {
    let lines = wrap_note_text(&note.text);
    let longest = lines
        .iter()
        .map(|l| l.chars().count())
        .max()
        .unwrap_or(0)
        .max(4);
    Rect {
        x: note.pos[0],
        y: note.pos[1],
        w: longest as f32 * NOTE_CHAR_WIDTH + NOTE_PADDING * 2.0,
        h: lines.len() as f32 * NOTE_LINE_HEIGHT + NOTE_PADDING * 2.0,
    }
}

// ── Endpoint resolution and hit-testing ──────────────────────────────────────

use crate::project::Endpoint;

/// Resolve a segment endpoint to (position, exit direction) against the
/// current sheet. Returns None if the endpoint references something that
/// no longer exists (callers treat such segments as invisible; the editor's
/// normalize pass removes them from the document).
pub fn resolve_endpoint(
    abstractions: &[ProjectAbstraction],
    components: &[ProjectComponent],
    interface: Option<&ProjectAbstraction>,
    junctions: &[Junction],
    endpoint: &Endpoint,
) -> Option<([f32; 2], ExitDir)> {
    match endpoint {
        Endpoint::Port {
            component_id,
            port_name,
        } => {
            let port = crate::project::PortRef {
                component_id: component_id.clone(),
                port_name: port_name.clone(),
            };
            let (pos, side) = port_geom_any(abstractions, components, interface, &port)?;
            Some((pos, Some(side)))
        }
        Endpoint::Junction { junction_id } => {
            let j = junctions.iter().find(|j| &j.id == junction_id)?;
            Some((j.pos, None))
        }
    }
}

/// The routed polyline of one segment, or None if an endpoint is dangling.
pub fn route_segment(
    abstractions: &[ProjectAbstraction],
    components: &[ProjectComponent],
    interface: Option<&ProjectAbstraction>,
    net: &WireNet,
    segment_index: usize,
) -> Option<Vec<[f32; 2]>> {
    let seg = net.segments.get(segment_index)?;
    let (a, a_dir) =
        resolve_endpoint(abstractions, components, interface, &net.junctions, &seg.from)?;
    let (b, b_dir) =
        resolve_endpoint(abstractions, components, interface, &net.junctions, &seg.to)?;
    Some(route(a, a_dir, b, b_dir))
}

/// What lies under a point on the sheet, in priority order: ports (including
/// interface pins) beat pin name-plates beat component bodies beat wires.
#[derive(Debug, Clone, PartialEq)]
pub enum Hit {
    Port {
        component_id: String,
        port_name: String,
        kind: PortKind,
    },
    /// The name-plate of an interface pin (click to select, Delete removes).
    Pin {
        name: String,
        kind: PortKind,
    },
    /// A wire node: click to draw from it, drag to move, Delete removes.
    Junction {
        net_id: String,
        junction_id: String,
    },
    Component {
        component_id: String,
    },
    Wire {
        net_id: String,
        segment_index: usize,
    },
    Note {
        note_id: String,
    },
    Empty,
}

pub fn hit_test(
    abstractions: &[ProjectAbstraction],
    components: &[ProjectComponent],
    wire_nets: &[WireNet],
    notes: &[crate::project::Note],
    interface: Option<&ProjectAbstraction>,
    p: [f32; 2],
) -> Hit {
    // Ports first: they sit on component edges and must win over bodies.
    // Iterate in reverse so the most recently placed component wins ties.
    for comp in components.iter().rev() {
        for port in component_ports(abstractions, comp) {
            if dist(p, port.pos) <= PORT_HIT_RADIUS {
                return Hit::Port {
                    component_id: comp.id.clone(),
                    port_name: port.info.name,
                    kind: port.info.kind,
                };
            }
        }
    }

    if let Some(def) = interface {
        for pin in interface_pins(abstractions, def) {
            if dist(p, pin.pos) <= PORT_HIT_RADIUS {
                let port = pin.port_ref();
                return Hit::Port {
                    component_id: port.component_id,
                    port_name: port.port_name,
                    kind: pin.wire_kind(),
                };
            }
            if pin.plate.contains(p) {
                return Hit::Pin {
                    name: pin.name,
                    kind: pin.io,
                };
            }
        }
    }

    // Wire nodes beat component bodies: a node routed over a chip must stay
    // grabbable.
    for net in wire_nets.iter().rev() {
        for junction in net.junctions.iter().rev() {
            if dist(p, junction.pos) <= JUNCTION_HIT_RADIUS {
                return Hit::Junction {
                    net_id: net.id.clone(),
                    junction_id: junction.id.clone(),
                };
            }
        }
    }

    for comp in components.iter().rev() {
        if footprint(abstractions, comp).contains(p) {
            return Hit::Component {
                component_id: comp.id.clone(),
            };
        }
    }

    for net in wire_nets.iter().rev() {
        for (i, _) in net.segments.iter().enumerate() {
            if let Some(pts) = route_segment(abstractions, components, interface, net, i) {
                if polyline_dist(&pts, p) <= WIRE_HIT_RADIUS {
                    return Hit::Wire {
                        net_id: net.id.clone(),
                        segment_index: i,
                    };
                }
            }
        }
    }

    // Notes sit behind everything interactive.
    for note in notes.iter().rev() {
        if note_bounds(note).contains(p) {
            return Hit::Note {
                note_id: note.id.clone(),
            };
        }
    }

    Hit::Empty
}

fn dist(a: [f32; 2], b: [f32; 2]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
}

/// Minimum distance from point `p` to a polyline.
pub fn polyline_dist(pts: &[[f32; 2]], p: [f32; 2]) -> f32 {
    let mut best = f32::INFINITY;
    for pair in pts.windows(2) {
        best = best.min(segment_dist(pair[0], pair[1], p));
    }
    best
}

fn segment_dist(a: [f32; 2], b: [f32; 2], p: [f32; 2]) -> f32 {
    let ab = [b[0] - a[0], b[1] - a[1]];
    let len_sq = ab[0] * ab[0] + ab[1] * ab[1];
    if len_sq < 1e-12 {
        return dist(a, p);
    }
    let t = (((p[0] - a[0]) * ab[0] + (p[1] - a[1]) * ab[1]) / len_sq).clamp(0.0, 1.0);
    dist([a[0] + ab[0] * t, a[1] + ab[1] * t], p)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::{PortRef, Segment};
    use std::collections::HashMap;

    fn comp(id: &str, ty: &str, x: f32, y: f32) -> ProjectComponent {
        ProjectComponent {
            id: id.into(),
            component_type: ty.into(),
            init: None,
            pos: [x, y],
        }
    }

    fn abs_not() -> ProjectAbstraction {
        ProjectAbstraction {
            pin_positions: HashMap::new(),
            name: "not".into(),
            inputs: vec!["x".into()],
            outputs: HashMap::from([("out".into(), "gate".into())]),
            width: 4.0,
            height: 2.0,
            components: vec![comp("gate", "nand", 0.0, 0.0)],
            wire_nets: vec![],
        notes: vec![],
        }
    }

    #[test]
    fn primitive_ports() {
        let ports = type_ports(&[], "nand").unwrap();
        assert_eq!(ports.len(), 3);
        assert_eq!(ports[0].name, "a");
        assert_eq!(ports[2].kind, PortKind::Output);
        assert!(type_ports(&[], "bogus").is_none());
    }

    #[test]
    fn abstraction_ports_are_deterministic() {
        let abs = vec![abs_not()];
        let ports = type_ports(&abs, "not").unwrap();
        assert_eq!(ports.len(), 2);
        assert_eq!(ports[0].name, "x");
        assert_eq!(ports[1].name, "out");
    }

    #[test]
    fn nand_port_geometry() {
        let c = comp("n1", "nand", 10.0, 20.0);
        let ports = component_ports(&[], &c);
        let a = ports.iter().find(|p| p.info.name == "a").unwrap();
        let b = ports.iter().find(|p| p.info.name == "b").unwrap();
        let out = ports.iter().find(|p| p.info.name == "out").unwrap();
        assert_eq!(a.pos, [10.0, 20.5]);
        assert_eq!(b.pos, [10.0, 21.5]);
        assert_eq!(out.pos, [13.0, 21.0]);
        assert_eq!(out.side, Side::Right);
    }

    #[test]
    fn footprints_intersect() {
        let a = Rect { x: 0.0, y: 0.0, w: 3.0, h: 2.0 };
        let b = Rect { x: 2.0, y: 1.0, w: 3.0, h: 2.0 };
        let c = Rect { x: 3.0, y: 0.0, w: 3.0, h: 2.0 };
        assert!(a.intersects(&b));
        assert!(!a.intersects(&c)); // touching edges is not overlap
    }

    #[test]
    fn route_straight_line() {
        let pts = route([3.0, 1.0], Some(Side::Right), [8.0, 1.0], Some(Side::Left));
        assert_eq!(pts.first(), Some(&[3.0, 1.0]));
        assert_eq!(pts.last(), Some(&[8.0, 1.0]));
        // Same y all the way: straight run.
        assert!(pts.iter().all(|p| p[1] == 1.0));
    }

    #[test]
    fn route_elbow_forward() {
        let pts = route([3.0, 1.0], Some(Side::Right), [10.0, 5.0], Some(Side::Left));
        // Orthogonal: every consecutive pair shares x or y.
        for pair in pts.windows(2) {
            assert!(
                pair[0][0] == pair[1][0] || pair[0][1] == pair[1][1],
                "non-orthogonal step {:?}",
                pair
            );
        }
        assert_eq!(pts.first(), Some(&[3.0, 1.0]));
        assert_eq!(pts.last(), Some(&[10.0, 5.0]));
    }

    #[test]
    fn route_backward_arcs_over() {
        // Target is to the LEFT of an output that exits right.
        let pts = route([10.0, 1.0], Some(Side::Right), [2.0, 5.0], Some(Side::Left));
        for pair in pts.windows(2) {
            assert!(pair[0][0] == pair[1][0] || pair[0][1] == pair[1][1]);
        }
        // First step must still exit rightward.
        assert!(pts[1][0] > pts[0][0]);
        // Last step must approach the input from its left.
        let n = pts.len();
        assert!(pts[n - 2][0] < pts[n - 1][0]);
        // The detour goes ABOVE both endpoints, not between them.
        assert!(pts.iter().any(|p| p[1] < 1.0));
    }

    #[test]
    fn route_backward_same_height_never_doubles_back() {
        // Output at x=10 feeding an input at x=2 on the SAME row: a naive
        // straight line would run backward through both components.
        let pts = route([10.0, 1.0], Some(Side::Right), [2.0, 1.0], Some(Side::Left));
        for pair in pts.windows(2) {
            assert!(pair[0][0] == pair[1][0] || pair[0][1] == pair[1][1]);
        }
        assert!(pts[1][0] > pts[0][0], "must exit rightward first");
        assert!(pts.iter().any(|p| p[1] < 1.0), "must arc around");
        let n = pts.len();
        assert!(pts[n - 2][0] < pts[n - 1][0], "must arrive from the left");
    }

    #[test]
    fn route_junction_into_port_respects_port_side() {
        // A junction RIGHT of an input port: the wire must swing around to
        // approach the port from its left, not stab it from the right.
        let pts = route([8.0, 1.0], None, [2.0, 1.0], Some(Side::Left));
        let n = pts.len();
        assert!(pts[n - 2][0] < pts[n - 1][0], "arrive traveling rightward");
    }

    #[test]
    fn hit_priority_port_over_body() {
        let comps = vec![comp("n1", "nand", 0.0, 0.0)];
        // Exactly on the "a" port.
        let hit = hit_test(&[], &comps, &[], &[], None, [0.0, 0.5]);
        assert!(matches!(hit, Hit::Port { ref port_name, .. } if port_name == "a"));
        // Center of the body.
        let hit = hit_test(&[], &comps, &[], &[], None, [1.5, 1.0]);
        assert!(matches!(hit, Hit::Component { ref component_id } if component_id == "n1"));
        // Far away.
        assert_eq!(hit_test(&[], &comps, &[], &[], None, [20.0, 20.0]), Hit::Empty);
    }

    #[test]
    fn hit_wire_segment() {
        let comps = vec![comp("c1", "clk", 0.0, 0.0), comp("n1", "nand", 6.0, 0.0)];
        let net = WireNet {
            id: "w1".into(),
            source: Some(PortRef {
                component_id: "c1".into(),
                port_name: "out".into(),
            }),
            sinks: vec![PortRef {
                component_id: "n1".into(),
                port_name: "a".into(),
            }],
            junctions: vec![],
            segments: vec![Segment {
                from: Endpoint::Port {
                    component_id: "c1".into(),
                    port_name: "out".into(),
                },
                to: Endpoint::Port {
                    component_id: "n1".into(),
                    port_name: "a".into(),
                },
            }],
        };
        // clk out is at (2, 1); nand a is at (6, 0.5). Midpoint of the run
        // sits near x=4 — probe a point on the wire.
        let pts = route_segment(&[], &comps, None, &net, 0).unwrap();
        let mid = pts[pts.len() / 2];
        let hit = hit_test(&[], &comps, &[net.clone()], &[], None, mid);
        assert!(matches!(hit, Hit::Wire { ref net_id, .. } if net_id == "w1"));
    }

    #[test]
    fn dangling_segment_resolves_to_none() {
        let net = WireNet {
            id: "w1".into(),
            source: None,
            sinks: vec![],
            junctions: vec![],
            segments: vec![Segment {
                from: Endpoint::Port {
                    component_id: "ghost".into(),
                    port_name: "out".into(),
                },
                to: Endpoint::Junction {
                    junction_id: "nope".into(),
                },
            }],
        };
        assert!(route_segment(&[], &[], None, &net, 0).is_none());
    }
}
