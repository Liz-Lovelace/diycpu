//! The wire graph. A net's SEGMENTS are the truth: one connected graph
//! of ports and junctions. The logical `source`/`sinks` the engine reads
//! are re-derived by `sync_net_logic` after every change; nets merge when
//! wires join them and split when cutting leaves islands. Bundle nets are
//! the same graph with stricter port rules.



use crate::layout::{self, PortKind};
use crate::project::{Endpoint, Junction, PortRef, Project, ProjectComponent, WireNet};

use super::*;


impl Editor {
    /// Connect two ports with a wire, optionally bending through `bends`
    /// (grid points, in order from `a` to `b`). One port must be an output
    /// and the other an input, in either argument order. If the input is
    /// already driven, the old connection is replaced.
    ///
    /// This is a convenience over [`Editor::add_wire`]: the whole path is
    /// one undo step. Inside an abstraction sheet, `INPUT:<name>` pins act
    /// as outputs and `OUTPUT:<name>` pins as inputs.
    pub fn connect(
        &mut self,
        a: PortRef,
        b: PortRef,
        bends: &[[f32; 2]],
    ) -> Result<(), EditError> {
        let bends = bends.to_vec();
        self.mutate(move |project, sheet| {
            let ctx = KindCtx::new(project, sheet)?;
            let (components, _) = sheet_of(project, sheet).ok_or(EditError::BadSheet)?;

            // Draw from the output side so bend order matches wire flow.
            let (from, to, bends) = match (
                ctx.kind_of(components, &a)?,
                ctx.kind_of(components, &b)?,
            ) {
                (PortKind::Output, PortKind::Input) => (a, b, bends),
                (PortKind::Input, PortKind::Output) => {
                    let mut rev = bends.clone();
                    rev.reverse();
                    (b, a, rev)
                }
                _ => return Err(EditError::NotAnInputAndOutput),
            };

            let mut anchor = WirePoint::Port(from);
            for bend in &bends {
                let attach = attach_wire(project, sheet, &anchor, &WirePoint::New(*bend))?;
                anchor = WirePoint::Junction {
                    net_id: attach.net_id,
                    junction_id: attach
                        .new_junction
                        .expect("New points always create a junction"),
                };
            }
            attach_wire(project, sheet, &anchor, &WirePoint::Port(to))?;
            guard_no_combinational_loop(project, sheet)?;
            Ok(())
        })
    }

    /// One wire-drawing step: join two points with a straight segment.
    /// Points can be ports, existing wire nodes, or a new node at a grid
    /// position. Nets are created, extended and merged as needed; wiring an
    /// already-connected input replaces its old connection. Each call is one
    /// undo step, so Ctrl+Z peels wires back click by click.
    pub fn add_wire(&mut self, from: WirePoint, to: WirePoint) -> Result<WireAttach, EditError> {
        self.mutate(move |project, sheet| {
            let attach = attach_wire(project, sheet, &from, &to)?;
            guard_no_combinational_loop(project, sheet)?;
            Ok(attach)
        })
    }

    /// Move a wire node to a new grid position.
    pub fn move_junction(
        &mut self,
        net_id: &str,
        junction_id: &str,
        pos: [f32; 2],
    ) -> Result<(), EditError> {
        {
            // A no-op move should not spend an undo step.
            let (_, wire_nets) = self.sheet_data();
            let current = wire_nets
                .iter()
                .find(|n| n.id == net_id)
                .and_then(|n| n.junctions.iter().find(|j| j.id == junction_id))
                .map(|j| j.pos);
            if current == Some(pos) {
                return Ok(());
            }
        }
        let net_id = net_id.to_string();
        let junction_id = junction_id.to_string();
        self.mutate(move |project, sheet| {
            let (_, wire_nets) = sheet_mut(project, sheet).ok_or(EditError::BadSheet)?;
            let junction = wire_nets
                .iter_mut()
                .find(|n| n.id == net_id)
                .and_then(|n| n.junctions.iter_mut().find(|j| j.id == junction_id))
                .ok_or_else(|| EditError::UnknownJunction(junction_id.clone()))?;
            junction.pos = pos;
            Ok(())
        })
    }

    /// Delete a wire node together with the segments that met there. If that
    /// cuts the net into disconnected pieces, each piece becomes its own net.
    pub fn delete_junction(&mut self, net_id: &str, junction_id: &str) -> Result<(), EditError> {
        let net_id = net_id.to_string();
        let junction_id = junction_id.to_string();
        self.mutate(move |project, sheet| {
            let ctx = KindCtx::new(project, sheet)?;
            let (components, wire_nets) =
                sheet_mut(project, sheet).ok_or(EditError::BadSheet)?;
            let net = wire_nets
                .iter_mut()
                .find(|n| n.id == net_id)
                .ok_or_else(|| EditError::UnknownJunction(junction_id.clone()))?;
            if !net.junctions.iter().any(|j| j.id == junction_id) {
                return Err(EditError::UnknownJunction(junction_id.clone()));
            }
            net.junctions.retain(|j| j.id != junction_id);
            net.segments.retain(|seg| {
                ![&seg.from, &seg.to].iter().any(|e| {
                    matches!(e, Endpoint::Junction { junction_id: id } if *id == junction_id)
                })
            });
            repair_nets(wire_nets, components, &ctx)?;
            Ok(())
        })
    }

    /// Delete one wire segment. If that cuts the net into disconnected
    /// pieces, each piece becomes its own net.
    pub fn delete_segment(
        &mut self,
        net_id: &str,
        segment_index: usize,
    ) -> Result<(), EditError> {
        let net_id = net_id.to_string();
        self.mutate(move |project, sheet| {
            let ctx = KindCtx::new(project, sheet)?;
            let (components, wire_nets) =
                sheet_mut(project, sheet).ok_or(EditError::BadSheet)?;
            let net = wire_nets
                .iter_mut()
                .find(|n| n.id == net_id)
                .ok_or_else(|| EditError::UnknownNet(net_id.clone()))?;
            if segment_index >= net.segments.len() {
                return Err(EditError::NoopWire);
            }
            net.segments.remove(segment_index);
            repair_nets(wire_nets, components, &ctx)?;
            Ok(())
        })
    }

    /// Split a segment by inserting a node at `pos` — the wire then runs
    /// through it and new branches can grow from it. Returns the node's id.
    pub fn insert_junction(
        &mut self,
        net_id: &str,
        segment_index: usize,
        pos: [f32; 2],
    ) -> Result<String, EditError> {
        let net_id = net_id.to_string();
        self.mutate(move |project, sheet| {
            let (_, wire_nets) = sheet_mut(project, sheet).ok_or(EditError::BadSheet)?;
            let net = wire_nets
                .iter_mut()
                .find(|n| n.id == net_id)
                .ok_or_else(|| EditError::UnknownNet(net_id.clone()))?;
            if segment_index >= net.segments.len() {
                return Err(EditError::NoopWire);
            }
            let seg = net.segments.remove(segment_index);
            let junction_id = generate_junction_id(net);
            net.junctions.push(Junction {
                id: junction_id.clone(),
                pos,
            });
            let mid = Endpoint::Junction {
                junction_id: junction_id.clone(),
            };
            net.segments.push(crate::project::Segment {
                from: seg.from,
                to: mid.clone(),
            });
            net.segments.push(crate::project::Segment {
                from: mid,
                to: seg.to,
            });
            Ok(junction_id)
        })
    }

    /// Remove one input's connection (the rest of the net stays, including
    /// any nodes that now dangle as reusable stubs).
    pub fn disconnect_sink(&mut self, sink: PortRef) -> Result<(), EditError> {
        self.mutate(move |project, sheet| {
            let ctx = KindCtx::new(project, sheet)?;
            let (components, wire_nets) =
                sheet_mut(project, sheet).ok_or(EditError::BadSheet)?;
            detach_port(wire_nets, &sink);
            repair_nets(wire_nets, components, &ctx)?;
            Ok(())
        })
    }

    /// Delete several wire segments (one undo step). Nets that fall apart
    /// split into their live pieces.
    pub fn delete_segments(&mut self, items: &[(String, usize)]) -> Result<(), EditError> {
        self.delete_mixed(&[], &[], &[], items, &[])
    }

    /// Unwire output pins (their derived wires disappear; the pins stay).
    pub fn unwire_output_pins(&mut self, names: &[String]) -> Result<(), EditError> {
        self.delete_mixed(&[], &[], &[], &[], names)
    }
}

/// A point a wire-drawing step can start or end on.
#[derive(Debug, Clone, PartialEq)]
pub enum WirePoint {
    Port(PortRef),
    Junction {
        net_id: String,
        junction_id: String,
    },
    /// Create a new node at this grid position.
    New([f32; 2]),
}

/// What an [`Editor::add_wire`] step produced.
#[derive(Debug, Clone)]
pub struct WireAttach {
    /// The net the segment landed in. Empty for output-pin connections
    /// (those live in the abstraction's outputs map, not in a net).
    pub net_id: String,
    /// The node created for a `WirePoint::New` target.
    pub new_junction: Option<String>,
}

pub(crate) fn port_endpoint(port: &PortRef) -> Endpoint {
    Endpoint::Port {
        component_id: port.component_id.clone(),
        port_name: port.port_name.clone(),
    }
}

pub(crate) fn endpoint_is_port(e: &Endpoint, port: &PortRef) -> bool {
    matches!(e, Endpoint::Port { component_id, port_name }
        if *component_id == port.component_id && *port_name == port.port_name)
}

pub(crate) fn net_touches_port(net: &WireNet, port: &PortRef) -> bool {
    net.segments
        .iter()
        .any(|seg| endpoint_is_port(&seg.from, port) || endpoint_is_port(&seg.to, port))
}

/// The single wire-drawing step both `add_wire` and `connect` build on.
pub(crate) fn attach_wire(
    project: &mut Project,
    sheet: SheetRef,
    from: &WirePoint,
    to: &WirePoint,
) -> Result<WireAttach, EditError> {
    let ctx = KindCtx::new(project, sheet)?;

    // Output-pin connections are outputs-map writes, not net edits —
    // except BUNDLE output pins, which take real bundle nets (the lowering
    // pass reads their connectivity from the net graph).
    let is_output_pin = |wp: &WirePoint| {
        matches!(wp, WirePoint::Port(p)
            if p.component_id.strip_prefix(layout::OUTPUT_PREFIX)
                .is_some_and(|name| !ctx.interface_bundles.contains_key(name)))
    };
    if is_output_pin(from) || is_output_pin(to) {
        return attach_to_output_pin(project, sheet, &ctx, from, to);
    }

    if from == to {
        return Err(EditError::NoopWire);
    }

    let (components, wire_nets) = sheet_mut(project, sheet).ok_or(EditError::BadSheet)?;

    // If this exact wire already exists, it's a no-op — checked before the
    // replace rule below can detach and re-add it.
    let existing_endpoint = |wp: &WirePoint| -> Option<Endpoint> {
        match wp {
            WirePoint::Port(p) => Some(port_endpoint(p)),
            WirePoint::Junction { junction_id, .. } => Some(Endpoint::Junction {
                junction_id: junction_id.clone(),
            }),
            WirePoint::New(_) => None,
        }
    };
    if let (Some(a), Some(b)) = (existing_endpoint(from), existing_endpoint(to)) {
        let already_there = wire_nets.iter().any(|n| {
            n.segments
                .iter()
                .any(|seg| (seg.from == a && seg.to == b) || (seg.from == b && seg.to == a))
        });
        if already_there {
            return Err(EditError::NoopWire);
        }
    }

    // Validate port points and apply the replace rule: wiring an input that
    // is already connected replaces its old connection.
    for wp in [from, to] {
        if let WirePoint::Port(p) = wp {
            if ctx.kind_of(components, p)? == PortKind::Input
                && wire_nets.iter().any(|n| net_touches_port(n, p))
            {
                detach_port(wire_nets, p);
            }
        }
    }

    // Resolve which existing net each point belongs to.
    let resolve_net = |wire_nets: &Vec<WireNet>, wp: &WirePoint| -> Result<Option<usize>, EditError> {
        match wp {
            WirePoint::Port(p) => Ok(wire_nets.iter().position(|n| net_touches_port(n, p))),
            WirePoint::Junction {
                net_id,
                junction_id,
            } => {
                let index = wire_nets
                    .iter()
                    .position(|n| {
                        n.id == *net_id && n.junctions.iter().any(|j| j.id == *junction_id)
                    })
                    .ok_or_else(|| EditError::UnknownJunction(junction_id.clone()))?;
                Ok(Some(index))
            }
            WirePoint::New(_) => Ok(None),
        }
    };

    let net_from = resolve_net(wire_nets, from)?;
    let net_to = resolve_net(wire_nets, to)?;

    // Junction ids can be renamed if a merge collides; track that.
    let mut renames: Vec<(String, String)> = Vec::new();
    let net_index = match (net_from, net_to) {
        (Some(a), Some(b)) if a != b => {
            if wire_nets[a].bundle != wire_nets[b].bundle {
                return Err(EditError::MixedWireKinds);
            }
            // Merging two live nets: at most one may be driven.
            if let (Some(sa), Some(sb)) = (&wire_nets[a].source, &wire_nets[b].source) {
                if sa != sb {
                    return Err(EditError::TwoSources(format!(
                        "{}.{}",
                        sa.component_id, sa.port_name
                    )));
                }
            }
            let absorbed = wire_nets.remove(b);
            let a = if b < a { a - 1 } else { a };
            renames = merge_net(&mut wire_nets[a], absorbed);
            a
        }
        (Some(a), _) => a,
        (_, Some(b)) => b,
        (None, None) => {
            // A fresh net's lane comes from the ports starting it; mixing
            // is rejected in sync either way.
            let mut bundle = false;
            for wp in [from, to] {
                if let WirePoint::Port(p) = wp {
                    if let Ok(layout::WireKind::Bundle(_)) =
                        ctx.wire_kind_of(components, p)
                    {
                        bundle = true;
                    }
                }
            }
            let id = generate_net_id(wire_nets);
            wire_nets.push(WireNet {
                bundle,
                id,
                source: None,
                sinks: vec![],
                junctions: vec![],
                segments: vec![],
            });
            wire_nets.len() - 1
        }
    };

    let mut new_junction = None;
    let mut endpoint_of = |wp: &WirePoint, net: &mut WireNet| -> Result<Endpoint, EditError> {
        match wp {
            WirePoint::Port(p) => Ok(port_endpoint(p)),
            WirePoint::Junction { junction_id, .. } => {
                let id = renames
                    .iter()
                    .find(|(old, _)| old == junction_id)
                    .map(|(_, new)| new.clone())
                    .unwrap_or_else(|| junction_id.clone());
                if !net.junctions.iter().any(|j| j.id == id) {
                    return Err(EditError::UnknownJunction(id));
                }
                Ok(Endpoint::Junction { junction_id: id })
            }
            WirePoint::New(pos) => {
                let id = generate_junction_id(net);
                net.junctions.push(Junction {
                    id: id.clone(),
                    pos: *pos,
                });
                new_junction = Some(id.clone());
                Ok(Endpoint::Junction { junction_id: id })
            }
        }
    };

    let net = &mut wire_nets[net_index];
    let from_ep = endpoint_of(from, net)?;
    let to_ep = endpoint_of(to, net)?;

    let duplicate = net.segments.iter().any(|seg| {
        (seg.from == from_ep && seg.to == to_ep) || (seg.from == to_ep && seg.to == from_ep)
    });
    if duplicate {
        return Err(EditError::NoopWire);
    }
    net.segments.push(crate::project::Segment {
        from: from_ep,
        to: to_ep,
    });

    let net_id = net.id.clone();
    // Re-derive logic; this is also where two-sources and doubly-wired
    // inputs are rejected (the whole step rolls back on error).
    for net in wire_nets.iter_mut() {
        sync_net_logic(net, components, &ctx)?;
        drop_orphan_junctions(net);
    }
    gc_nets(wire_nets);

    Ok(WireAttach {
        net_id,
        new_junction,
    })
}

/// Wiring into an `OUTPUT:` pin records the driving port in the
/// abstraction's outputs map (the wire itself is drawn derived from it).
pub(crate) fn attach_to_output_pin(
    project: &mut Project,
    sheet: SheetRef,
    ctx: &KindCtx,
    from: &WirePoint,
    to: &WirePoint,
) -> Result<WireAttach, EditError> {
    let SheetRef::Abstraction(index) = sheet else {
        return Err(EditError::NotAnAbstractionSheet);
    };
    let is_pin = |wp: &WirePoint| {
        matches!(wp, WirePoint::Port(p) if p.component_id.starts_with(layout::OUTPUT_PREFIX))
    };
    let (pin, other) = if is_pin(from) { (from, to) } else { (to, from) };
    let WirePoint::Port(pin_port) = pin else {
        unreachable!("is_pin checked")
    };
    let pin_name = pin_port
        .component_id
        .strip_prefix(layout::OUTPUT_PREFIX)
        .expect("checked prefix")
        .to_string();

    let (components, wire_nets) = sheet_mut(project, sheet).ok_or(EditError::BadSheet)?;
    if !ctx
        .interface_outputs
        .as_ref()
        .is_some_and(|outs| outs.contains(&pin_name))
    {
        return Err(EditError::UnknownComponent(pin_port.component_id.clone()));
    }

    // Find the port that will drive this pin.
    let source_port: PortRef = match other {
        WirePoint::Port(p) => {
            // kind_of also validates that an INPUT: pseudo-port names a
            // declared input pin.
            if ctx.kind_of(components, p)? != PortKind::Output {
                return Err(EditError::NotAnInputAndOutput);
            }
            p.clone()
        }
        WirePoint::Junction { net_id, .. } => {
            let net = wire_nets
                .iter()
                .find(|n| &n.id == net_id)
                .ok_or_else(|| EditError::UnknownJunction(net_id.clone()))?;
            net.source
                .clone()
                .ok_or(EditError::OutputPinNeedsSource)?
        }
        WirePoint::New(_) => return Err(EditError::OutputPinNeedsSource),
    };

    // An input pin wired straight into an output pin is a passthrough:
    // the output simply forwards that input.
    let target = if source_port.component_id.starts_with(crate::convert::INPUT_PREFIX) {
        source_port.component_id.clone()
    } else {
        let source_comp = components
            .iter()
            .find(|c| c.id == source_port.component_id)
            .ok_or_else(|| EditError::UnknownComponent(source_port.component_id.clone()))?;
        if crate::types::is_primitive(&source_comp.component_type) {
            source_comp.id.clone()
        } else {
            format!("{}.{}", source_port.component_id, source_port.port_name)
        }
    };
    project.abstractions[index].outputs.insert(pin_name, target);
    Ok(WireAttach {
        net_id: String::new(),
        new_junction: None,
    })
}

/// Remove every segment touching `port`, across all nets. Orphaned nodes and
/// empty nets are left for `repair_nets`/callers to tidy.
pub(crate) fn detach_port(wire_nets: &mut [WireNet], port: &PortRef) {
    for net in wire_nets.iter_mut() {
        net.segments
            .retain(|seg| !endpoint_is_port(&seg.from, port) && !endpoint_is_port(&seg.to, port));
    }
}

/// Derive a net's logical `source`/`sinks` from its segment endpoints.
/// Rejects nets with two different driving outputs or an input wired twice.
pub(crate) fn sync_net_logic(
    net: &mut WireNet,
    components: &[ProjectComponent],
    ctx: &KindCtx,
) -> Result<(), EditError> {
    let mut source: Option<PortRef> = None;
    let mut sinks: Vec<PortRef> = Vec::new();
    let mut lane_width: Option<u32> = None;
    for seg in &net.segments {
        for e in [&seg.from, &seg.to] {
            let Endpoint::Port {
                component_id,
                port_name,
            } = e
            else {
                continue;
            };
            let port = PortRef {
                component_id: component_id.clone(),
                port_name: port_name.clone(),
            };
            // Bundle nets take only bundle ports (of one width); signal
            // nets take only signal ports. The two lanes never mix.
            match (net.bundle, ctx.wire_kind_of(components, &port)?) {
                (false, layout::WireKind::Signal) => {}
                (true, layout::WireKind::Bundle(w)) => match lane_width {
                    None => lane_width = Some(w),
                    Some(prev) if prev != w => {
                        return Err(EditError::BundleWidthMismatch(prev, w))
                    }
                    _ => {}
                },
                _ => return Err(EditError::MixedWireKinds),
            }
            match ctx.kind_of(components, &port)? {
                PortKind::Output => match &source {
                    None => source = Some(port),
                    Some(s) if *s == port => {}
                    Some(s) => {
                        return Err(EditError::TwoSources(format!(
                            "{}.{}",
                            s.component_id, s.port_name
                        )))
                    }
                },
                PortKind::Input => {
                    if sinks.contains(&port) {
                        return Err(EditError::InputDoublyWired(format!(
                            "{}.{}",
                            port.component_id, port.port_name
                        )));
                    }
                    sinks.push(port);
                }
            }
        }
    }
    net.source = source;
    net.sinks = sinks;
    Ok(())
}

/// Drop nodes no segment touches (invisible floating dots).
pub(crate) fn drop_orphan_junctions(net: &mut WireNet) {
    net.junctions.retain(|j| {
        net.segments.iter().any(|seg| {
            [&seg.from, &seg.to].iter().any(
                |e| matches!(e, Endpoint::Junction { junction_id } if *junction_id == j.id),
            )
        })
    });
}

pub(crate) fn endpoint_key(e: &Endpoint) -> String {
    match e {
        Endpoint::Port {
            component_id,
            port_name,
        } => format!("P:{component_id}\u{0}{port_name}"),
        Endpoint::Junction { junction_id } => format!("J:{junction_id}"),
    }
}

/// A net must be one connected graph: if its segments form several islands,
/// split the extras into their own nets.
pub(crate) fn split_all_nets(wire_nets: &mut Vec<WireNet>) {
    let mut i = 0;
    while i < wire_nets.len() {
        let bundle = wire_nets[i].bundle;
        for (junctions, segments) in split_off_islands(&mut wire_nets[i]) {
            let id = generate_net_id(wire_nets);
            wire_nets.push(WireNet {
                bundle,
                id,
                source: None, // synced by the caller
                sinks: vec![],
                junctions,
                segments,
            });
        }
        i += 1;
    }
}

/// Remove all but the first connected component from `net`, returning the
/// (junctions, segments) of each removed island.
pub(crate) fn split_off_islands(
    net: &mut WireNet,
) -> Vec<(Vec<Junction>, Vec<crate::project::Segment>)> {
    if net.segments.len() <= 1 {
        return vec![];
    }

    // Group segments into connected components over endpoint keys.
    let mut groups: Vec<(Vec<String>, Vec<crate::project::Segment>)> = Vec::new();
    for seg in net.segments.drain(..) {
        let ka = endpoint_key(&seg.from);
        let kb = endpoint_key(&seg.to);
        let touching: Vec<usize> = groups
            .iter()
            .enumerate()
            .filter(|(_, (keys, _))| keys.contains(&ka) || keys.contains(&kb))
            .map(|(g, _)| g)
            .collect();
        match touching.as_slice() {
            [] => groups.push((vec![ka, kb], vec![seg])),
            [g] => {
                let (keys, segs) = &mut groups[*g];
                keys.push(ka);
                keys.push(kb);
                segs.push(seg);
            }
            [first, rest @ ..] => {
                // This segment bridges several groups: merge them.
                for &g in rest.iter().rev() {
                    let (keys, segs) = groups.remove(g);
                    let target = &mut groups[*first];
                    target.0.extend(keys);
                    target.1.extend(segs);
                }
                let target = &mut groups[*first];
                target.0.push(ka);
                target.0.push(kb);
                target.1.push(seg);
            }
        }
    }

    // First group stays in this net; junctions follow their group.
    let all_junctions = std::mem::take(&mut net.junctions);
    let mut islands = Vec::new();
    for (index, (keys, segments)) in groups.into_iter().enumerate() {
        let junctions: Vec<Junction> = all_junctions
            .iter()
            .filter(|j| keys.contains(&format!("J:{}", j.id)))
            .cloned()
            .collect();
        if index == 0 {
            net.junctions = junctions;
            net.segments = segments;
        } else {
            islands.push((junctions, segments));
        }
    }
    islands
}

/// The standard post-removal consistency pass: drop floating nodes, split
/// disconnected islands into their own nets, re-derive logic, and collect
/// nets that lost all their segments.
pub(crate) fn repair_nets(
    wire_nets: &mut Vec<WireNet>,
    components: &[ProjectComponent],
    ctx: &KindCtx,
) -> Result<(), EditError> {
    for net in wire_nets.iter_mut() {
        drop_orphan_junctions(net);
    }
    split_all_nets(wire_nets);
    for net in wire_nets.iter_mut() {
        sync_net_logic(net, components, ctx)?;
    }
    gc_nets(wire_nets);
    Ok(())
}

/// A net exists as long as it has any wire in it. Stubs are legitimate;
/// only truly empty nets are collected.
pub(crate) fn gc_nets(wire_nets: &mut Vec<WireNet>) {
    wire_nets.retain(|n| !n.segments.is_empty());
}

/// Fold `absorbed` into `net`, renaming colliding junction IDs.
/// Returns the (old, new) id pairs of renamed junctions.
pub(crate) fn merge_net(net: &mut WireNet, mut absorbed: WireNet) -> Vec<(String, String)> {
    let mut renames = Vec::new();
    for junction in &mut absorbed.junctions {
        if net.junctions.iter().any(|j| j.id == junction.id) {
            let new_id = generate_junction_id(net);
            for seg in &mut absorbed.segments {
                for e in [&mut seg.from, &mut seg.to] {
                    if matches!(e, Endpoint::Junction { junction_id } if *junction_id == junction.id)
                    {
                        *e = Endpoint::Junction {
                            junction_id: new_id.clone(),
                        };
                    }
                }
            }
            renames.push((junction.id.clone(), new_id.clone()));
            junction.id = new_id;
        }
        net.junctions.push(junction.clone());
    }
    if net.source.is_none() {
        net.source = absorbed.source.take();
    }
    net.sinks.append(&mut absorbed.sinks);
    net.segments.append(&mut absorbed.segments);
    renames
}
