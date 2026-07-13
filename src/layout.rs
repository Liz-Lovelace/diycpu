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

/// What a port carries: one signal, or a whole bundle of them. Bundle
/// ports only accept bundle wires and vice versa.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WireKind {
    Signal,
    Bundle(u32),
}

#[derive(Debug, Clone)]
pub struct PortInfo {
    pub name: String,
    pub kind: PortKind,
    pub wire: WireKind,
}

/// Lane count of a bundler/unbundler (stored per component; default 8).
pub fn bits_of(comp: &ProjectComponent) -> u32 {
    comp.bits.unwrap_or(8).clamp(2, 32)
}

/// Is this a bundler or unbundler — the GUI-only bus components?
pub fn is_bundle_component(component_type: &str) -> bool {
    matches!(component_type, "bundle" | "unbundle")
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

    /// Smallest rectangle covering both.
    pub fn union(&self, other: &Rect) -> Rect {
        let x = self.x.min(other.x);
        let y = self.y.min(other.y);
        let x2 = (self.x + self.w).max(other.x + other.w);
        let y2 = (self.y + self.h).max(other.y + other.h);
        Rect { x, y, w: x2 - x, h: y2 - y }
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
        wire: WireKind::Signal,
    };
    match component_type {
        // Bundlers need the per-component lane count; this type-level view
        // assumes the default (callers with a component at hand use
        // `component_port_info`).
        "bundle" | "unbundle" => Some(bundle_ports(component_type, 8)),
        "clk" | "const" => Some(vec![port("out", PortKind::Output)]),
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
            let wire_of = |pin: &str| match def.bundle_pins.get(pin) {
                Some(w) => WireKind::Bundle(*w),
                None => WireKind::Signal,
            };
            let mut ports: Vec<PortInfo> = def
                .inputs
                .iter()
                .map(|i| PortInfo {
                    name: i.clone(),
                    kind: PortKind::Input,
                    wire: wire_of(i),
                })
                .collect();
            // HashMap order is unstable; sort output names for determinism.
            let mut outs: Vec<&String> = def.outputs.keys().collect();
            outs.sort();
            ports.extend(outs.into_iter().map(|o| PortInfo {
                name: o.clone(),
                kind: PortKind::Output,
                wire: wire_of(o),
            }));
            Some(ports)
        }
    }
}

/// The ports of a bundler/unbundler with `bits` lanes: numbered signal
/// ports on one side, the single bundle port on the other.
fn bundle_ports(component_type: &str, bits: u32) -> Vec<PortInfo> {
    let mut ports = Vec::new();
    let (lane_kind, bundle_kind) = if component_type == "bundle" {
        (PortKind::Input, PortKind::Output)
    } else {
        (PortKind::Output, PortKind::Input)
    };
    for k in 0..bits {
        ports.push(PortInfo {
            name: format!("{k}"),
            kind: lane_kind,
            wire: WireKind::Signal,
        });
    }
    ports.push(PortInfo {
        name: "bundle".into(),
        kind: bundle_kind,
        wire: WireKind::Bundle(bits),
    });
    ports
}

/// The port list of a specific component — like [`type_ports`], but aware
/// of per-component state (a bundler's lane count).
pub fn component_port_info(
    abstractions: &[ProjectAbstraction],
    comp: &ProjectComponent,
) -> Option<Vec<PortInfo>> {
    if is_bundle_component(&comp.component_type) {
        return Some(bundle_ports(&comp.component_type, bits_of(comp)));
    }
    type_ports(abstractions, &comp.component_type)
}

/// Footprint size in grid units for a component type. None for unknown types.
pub fn type_size(abstractions: &[ProjectAbstraction], component_type: &str) -> Option<[f32; 2]> {
    match component_type {
        "bundle" | "unbundle" => Some([2.0, 8.0]),
        "clk" | "const" => Some([2.0, 2.0]),
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
    let [w, h] = if is_bundle_component(&comp.component_type) {
        [2.0, bits_of(comp) as f32]
    } else {
        type_size(abstractions, &comp.component_type).unwrap_or([3.0, 2.0])
    };
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
    Top,
    Bottom,
}

impl Side {
    /// Outward unit normal of this side (the direction wires leave in).
    pub fn outward(self) -> [f32; 2] {
        match self {
            Side::Left => [-1.0, 0.0],
            Side::Right => [1.0, 0.0],
            Side::Top => [0.0, -1.0],
            Side::Bottom => [0.0, 1.0],
        }
    }

    pub fn from_project(side: crate::project::PortSide) -> Side {
        match side {
            crate::project::PortSide::Left => Side::Left,
            crate::project::PortSide::Right => Side::Right,
            crate::project::PortSide::Top => Side::Top,
            crate::project::PortSide::Bottom => Side::Bottom,
        }
    }

    pub fn to_project(self) -> crate::project::PortSide {
        match self {
            Side::Left => crate::project::PortSide::Left,
            Side::Right => crate::project::PortSide::Right,
            Side::Top => crate::project::PortSide::Top,
            Side::Bottom => crate::project::PortSide::Bottom,
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
    let Some(ports) = component_port_info(abstractions, comp) else {
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

    // Every port sits at a half-grid slot along its edge, so ports align
    // across all components. Custom spots snap to the nearest slot; autos
    // stack from a centered start.
    let custom = |name: &str| -> Option<(Side, f32)> {
        let def = abstractions.iter().find(|a| a.name == comp.component_type)?;
        let spot = def.port_layout.get(name)?;
        Some((Side::from_project(spot.side), spot.t.clamp(0.0, 1.0)))
    };
    let edge_len = |side: Side| match side {
        Side::Left | Side::Right => rect.h,
        Side::Top | Side::Bottom => rect.w,
    };
    // Snap a distance along an edge to the nearest x.5 slot inside it.
    let snap_slot = |offset: f32, len: f32| -> f32 {
        ((offset - 0.5).round() + 0.5).clamp(0.5, (len - 0.5).max(0.5))
    };
    let place = |side: Side, offset: f32| -> [f32; 2] {
        match side {
            Side::Left => [rect.x, rect.y + offset],
            Side::Right => [rect.x + rect.w, rect.y + offset],
            Side::Top => [rect.x + offset, rect.y],
            Side::Bottom => [rect.x + offset, rect.y + rect.h],
        }
    };

    let mut result = Vec::with_capacity(ports.len());
    for (list, default_side) in [(inputs, Side::Left), (outputs, Side::Right)] {
        let auto_count = list.iter().filter(|p| custom(&p.name).is_none()).count();
        let len = edge_len(default_side);
        let auto_start = ((len - auto_count as f32) / 2.0).floor().max(0.0);
        let mut auto_index = 0usize;
        for info in list {
            let (side, offset) = match custom(&info.name) {
                Some((side, t)) => {
                    let len = edge_len(side);
                    (side, snap_slot(t * len, len))
                }
                None => {
                    let offset = snap_slot(auto_start + 0.5 + auto_index as f32, len);
                    auto_index += 1;
                    (default_side, offset)
                }
            };
            result.push(PortGeom {
                info: info.clone(),
                pos: place(side, offset),
                side,
            });
        }
    }
    result
}

/// The (side, t) every port currently occupies — used to freeze the auto
/// layout in place the first time the user hand-places any port.
pub fn current_port_spots(
    abstractions: &[ProjectAbstraction],
    def: &ProjectAbstraction,
) -> Vec<(String, crate::project::PortSide, f32)> {
    let synthetic = ProjectComponent {
        bits: None,
        id: String::new(),
        component_type: def.name.clone(),
        init: None,
        pos: [0.0, 0.0],
    };
    component_ports(abstractions, &synthetic)
        .into_iter()
        .map(|g| {
            let (len, offset) = match g.side {
                Side::Left | Side::Right => (def.height, g.pos[1]),
                Side::Top | Side::Bottom => (def.width, g.pos[0]),
            };
            (g.info.name, g.side.to_project(), (offset / len).clamp(0.0, 1.0))
        })
        .collect()
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

// ── Wire geometry ────────────────────────────────────────────────────────────
//
// Wires have no auto-routing: a segment is a straight line between its two
// endpoints, diagonals included. Users route by placing nodes. Committed
// wires, the live drag preview, and hit-testing all share this one rule,
// so what you see while drawing is always what you get.

/// The polyline of a wire between two points: a straight line.
pub fn route(a: [f32; 2], b: [f32; 2]) -> Vec<[f32; 2]> {
    vec![a, b]
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
    /// Lane count when this pin carries a bundle.
    pub bundle: Option<u32>,
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
        bounds = Some(bounds.map_or(r, |b| b.union(&r)));
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
            bundle: def.bundle_pins.get(name).copied(),
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
            bundle: def.bundle_pins.get(name).copied(),
        });
    }
    pins
}

/// What a port reference on this sheet carries (signal or bundle),
/// including the pseudo-ports of interface pins. None for unknown refs.
pub fn wire_kind_any(
    abstractions: &[ProjectAbstraction],
    components: &[ProjectComponent],
    interface: Option<&ProjectAbstraction>,
    port: &crate::project::PortRef,
) -> Option<WireKind> {
    let pin_kind = |name: &str| {
        let def = interface?;
        Some(match def.bundle_pins.get(name) {
            Some(w) => WireKind::Bundle(*w),
            None => WireKind::Signal,
        })
    };
    if let Some(name) = port.component_id.strip_prefix(crate::convert::INPUT_PREFIX) {
        return pin_kind(name);
    }
    if let Some(name) = port.component_id.strip_prefix(OUTPUT_PREFIX) {
        return pin_kind(name);
    }
    let comp = components.iter().find(|c| c.id == port.component_id)?;
    component_port_info(abstractions, comp)?
        .into_iter()
        .find(|p| p.name == port.port_name)
        .map(|p| p.wire)
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
        bounds = Some(bounds.map_or(r, |b| b.union(&r)));
    };
    for note in &def.notes {
        extend(note_bounds(note));
    }
    for img in &def.images {
        extend(image_bounds(img));
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

// ── Images ───────────────────────────────────────────────────────────────────

/// The rectangle of a placed picture, in grid units.
pub fn image_bounds(img: &crate::project::MediaImage) -> Rect {
    Rect {
        x: img.pos[0],
        y: img.pos[1],
        w: img.size[0].max(0.5),
        h: img.size[1].max(0.5),
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
) -> Option<[f32; 2]> {
    match endpoint {
        Endpoint::Port {
            component_id,
            port_name,
        } => {
            let port = crate::project::PortRef {
                component_id: component_id.clone(),
                port_name: port_name.clone(),
            };
            let (pos, _side) = port_geom_any(abstractions, components, interface, &port)?;
            Some(pos)
        }
        Endpoint::Junction { junction_id } => {
            let j = junctions.iter().find(|j| &j.id == junction_id)?;
            Some(j.pos)
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
    let a = resolve_endpoint(abstractions, components, interface, &net.junctions, &seg.from)?;
    let b = resolve_endpoint(abstractions, components, interface, &net.junctions, &seg.to)?;
    Some(route(a, b))
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
    /// The derived wire feeding an output pin (click selects, Delete
    /// unwires the pin).
    OutputWire {
        pin: String,
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
    Image {
        image_id: String,
    },
    Empty,
}

pub fn hit_test(
    abstractions: &[ProjectAbstraction],
    components: &[ProjectComponent],
    wire_nets: &[WireNet],
    notes: &[crate::project::Note],
    images: &[crate::project::MediaImage],
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

    // Derived output-pin wires.
    if let Some(def) = interface {
        let pins = interface_pins(abstractions, def);
        for (name, target) in &def.outputs {
            if target.is_empty() {
                continue;
            }
            let Some(pin) = pins
                .iter()
                .find(|pg| pg.io == PortKind::Output && &pg.name == name)
            else {
                continue;
            };
            let port = output_target_port(target);
            let Some((from, _)) = port_geom_any(abstractions, components, interface, &port)
            else {
                continue;
            };
            if polyline_dist(&route(from, pin.pos), p) <= WIRE_HIT_RADIUS {
                return Hit::OutputWire { pin: name.clone() };
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

    // Pictures are the backdrop — behind even the notes.
    for img in images.iter().rev() {
        if image_bounds(img).contains(p) {
            return Hit::Image {
                image_id: img.id.clone(),
            };
        }
    }

    Hit::Empty
}

/// Does the segment a—b touch the rectangle at all (endpoint inside, or
/// crossing an edge)? Used by box-select so grabbing any part of a wire
/// grabs the wire — the same feel components have.
pub fn segment_intersects_rect(a: [f32; 2], b: [f32; 2], r: &Rect) -> bool {
    if r.contains(a) || r.contains(b) {
        return true;
    }
    let corners = [
        [r.x, r.y],
        [r.x + r.w, r.y],
        [r.x + r.w, r.y + r.h],
        [r.x, r.y + r.h],
    ];
    (0..4).any(|i| segments_cross(a, b, corners[i], corners[(i + 1) % 4]))
}

/// Proper or touching intersection of segments p1—p2 and p3—p4.
fn segments_cross(p1: [f32; 2], p2: [f32; 2], p3: [f32; 2], p4: [f32; 2]) -> bool {
    let cross = |a: [f32; 2], b: [f32; 2], c: [f32; 2]| {
        (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
    };
    let d1 = cross(p3, p4, p1);
    let d2 = cross(p3, p4, p2);
    let d3 = cross(p1, p2, p3);
    let d4 = cross(p1, p2, p4);
    if ((d1 > 0.0) != (d2 > 0.0)) && ((d3 > 0.0) != (d4 > 0.0)) {
        return true;
    }
    // Collinear/touching cases.
    let on = |a: [f32; 2], b: [f32; 2], c: [f32; 2], d: f32| {
        d.abs() < 1e-6
            && c[0] >= a[0].min(b[0]) - 1e-6
            && c[0] <= a[0].max(b[0]) + 1e-6
            && c[1] >= a[1].min(b[1]) - 1e-6
            && c[1] <= a[1].max(b[1]) + 1e-6
    };
    on(p3, p4, p1, d1) || on(p3, p4, p2, d2) || on(p1, p2, p3, d3) || on(p1, p2, p4, d4)
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
            bits: None,
            id: id.into(),
            component_type: ty.into(),
            init: None,
            pos: [x, y],
        }
    }

    fn abs_not() -> ProjectAbstraction {
        ProjectAbstraction {
            bundle_pins: HashMap::new(),
            pin_positions: HashMap::new(),
            port_layout: HashMap::new(),
            name: "not".into(),
            inputs: vec!["x".into()],
            outputs: HashMap::from([("out".into(), "gate".into())]),
            width: 4.0,
            height: 2.0,
            components: vec![comp("gate", "nand", 0.0, 0.0)],
            wire_nets: vec![],
        notes: vec![],
            images: vec![],
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
        assert_eq!(out.pos, [13.0, 20.5], "outputs sit on half-grid slots too");
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
    fn custom_port_layout_places_ports_on_any_edge_snapped() {
        use crate::project::{PortSide, PortSpot};
        let mut abs = abs_not();
        abs.port_layout
            .insert("x".into(), PortSpot { side: PortSide::Top, t: 0.5 });
        abs.port_layout
            .insert("out".into(), PortSpot { side: PortSide::Bottom, t: 0.25 });
        let abstractions = vec![abs];
        let c = comp("n1", "not", 10.0, 20.0); // 4x2 footprint
        let ports = component_ports(&abstractions, &c);
        let x = ports.iter().find(|p| p.info.name == "x").unwrap();
        assert_eq!(x.side, Side::Top);
        assert_eq!(x.pos, [12.5, 20.0], "snapped to a half-grid slot");
        let out = ports.iter().find(|p| p.info.name == "out").unwrap();
        assert_eq!(out.side, Side::Bottom);
        assert_eq!(out.pos, [11.5, 22.0]);
    }

    #[test]
    fn current_port_spots_round_trips_the_auto_layout() {
        // Freezing the auto layout must not move anything: spots derived
        // from the current geometry place ports exactly where they were.
        let mut abs = abs_not();
        abs.width = 4.0;
        abs.height = 3.0;
        let abstractions = vec![abs.clone()];
        let before = component_ports(&abstractions, &comp("n1", "not", 0.0, 0.0));
        let spots = current_port_spots(&abstractions, &abstractions[0]);
        let mut frozen = abs.clone();
        for (name, side, t) in spots {
            frozen
                .port_layout
                .insert(name, crate::project::PortSpot { side, t });
        }
        let frozen_list = vec![frozen];
        let after = component_ports(&frozen_list, &comp("n1", "not", 0.0, 0.0));
        for (b, a) in before.iter().zip(after.iter()) {
            assert_eq!(b.info.name, a.info.name);
            assert_eq!(b.pos, a.pos, "freeze moved port {}", b.info.name);
        }
    }

    #[test]
    fn wires_are_straight_lines() {
        assert_eq!(route([3.0, 1.0], [8.0, 4.0]), vec![[3.0, 1.0], [8.0, 4.0]]);
    }

    #[test]
    fn hit_priority_port_over_body() {
        let comps = vec![comp("n1", "nand", 0.0, 0.0)];
        // Exactly on the "a" port.
        let hit = hit_test(&[], &comps, &[], &[], &[], None, [0.0, 0.5]);
        assert!(matches!(hit, Hit::Port { ref port_name, .. } if port_name == "a"));
        // Center of the body.
        let hit = hit_test(&[], &comps, &[], &[], &[], None, [1.5, 1.0]);
        assert!(matches!(hit, Hit::Component { ref component_id } if component_id == "n1"));
        // Far away.
        assert_eq!(hit_test(&[], &comps, &[], &[], &[], None, [20.0, 20.0]), Hit::Empty);
    }

    #[test]
    fn hit_wire_segment() {
        let comps = vec![comp("c1", "clk", 0.0, 0.0), comp("n1", "nand", 6.0, 0.0)];
        let net = WireNet {
            bundle: false,
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
        // Probe the middle of the straight run (endpoints are ports).
        let mid = [
            (pts[0][0] + pts[1][0]) / 2.0,
            (pts[0][1] + pts[1][1]) / 2.0,
        ];
        let hit = hit_test(&[], &comps, std::slice::from_ref(&net), &[], &[], None, mid);
        assert!(matches!(hit, Hit::Wire { ref net_id, .. } if net_id == "w1"));
    }

    #[test]
    fn dangling_segment_resolves_to_none() {
        let net = WireNet {
            bundle: false,
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

#[cfg(test)]
mod band_tests {
    use super::*;

    #[test]
    fn segment_rect_intersection_matches_the_band_select_feel() {
        let band = Rect { x: 0.0, y: 0.0, w: 4.0, h: 4.0 };
        // Crossing straight through — endpoints both outside.
        assert!(segment_intersects_rect([-2.0, 2.0], [6.0, 2.0], &band));
        // Diagonal cutting a corner.
        assert!(segment_intersects_rect([3.0, -1.0], [6.0, 2.0], &band));
        // One endpoint inside.
        assert!(segment_intersects_rect([1.0, 1.0], [9.0, 9.0], &band));
        // Fully inside.
        assert!(segment_intersects_rect([1.0, 1.0], [2.0, 3.0], &band));
        // Clear miss alongside.
        assert!(!segment_intersects_rect([5.0, 0.0], [5.0, 4.0], &band));
        assert!(!segment_intersects_rect([-1.0, 5.0], [5.0, 5.0], &band));
    }
}
