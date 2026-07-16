//! The wire-graph stress battery: long randomized sequences of every kind
//! of edit, with the full invariant set checked after every single step.
//! When something here fails, the seed and step number reproduce it
//! exactly — turn the panic into a focused regression test.

use crate::layout::PortKind;
use crate::project::PortRef;

use super::sheets::check_invariants;
use super::*;

/// A tiny deterministic RNG (xorshift) — the battery must replay exactly.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
    fn chance(&mut self, percent: u64) -> bool {
        self.next() % 100 < percent
    }
}

/// Every port on the current sheet, pseudo-pins included.
fn all_ports(e: &Editor) -> Vec<PortRef> {
    let (components, _) = e.sheet_data();
    let abstractions = &e.project.abstractions;
    let mut out = Vec::new();
    for c in components {
        if let Some(ports) = crate::layout::component_port_info(abstractions, c) {
            for p in ports {
                out.push(PortRef {
                    component_id: c.id.clone(),
                    port_name: p.name,
                });
            }
        }
    }
    if let SheetRef::Abstraction(i) = e.sheet {
        if let Some(def) = e.project.abstractions.get(i) {
            for name in &def.inputs {
                out.push(PortRef {
                    component_id: format!("{}{name}", crate::convert::INPUT_PREFIX),
                    port_name: name.clone(),
                });
            }
            for name in def.outputs.keys() {
                out.push(PortRef {
                    component_id: format!("{}{name}", crate::convert::OUTPUT_PREFIX),
                    port_name: name.clone(),
                });
            }
        }
    }
    out
}

fn run_battery(seed: u64, steps: usize) {
    let mut rng = Rng(seed);
    let mut e = Editor::empty();

    // A stage with two abstractions to bounce between.
    let a = e.create_abstraction("alpha").unwrap();
    e.open_sheet(SheetRef::Abstraction(a)).unwrap();
    e.add_interface_pin(PortKind::Input, "x", None).unwrap();
    e.add_interface_pin(PortKind::Input, "d8", Some(8)).unwrap();
    e.add_interface_pin(PortKind::Output, "q", None).unwrap();
    e.add_interface_pin(PortKind::Output, "b8", Some(8)).unwrap();
    e.open_sheet(SheetRef::Top).unwrap();

    let types = ["nand", "dff", "const", "clk", "bundle", "unbundle", "light", "alpha"];
    let mut ok = 0usize;
    for step in 0..steps {
        let roll = rng.below(100);
        let t0 = std::time::Instant::now();
        // Any op may legitimately refuse (overlap, loop, mismatch…);
        // refusals must roll back cleanly, which the invariants verify.
        let outcome: Result<(), EditError> = (|| {
            match roll {
                // Place something somewhere — or, once the sheet is crowded,
                // delete instead (unbounded growth makes late steps crawl:
                // every op re-settles and re-checks the whole document).
                0..=19 => {
                    let (components, _) = e.sheet_data();
                    if components.len() >= 30 {
                        let id = components[rng.below(components.len())].id.clone();
                        return e.delete_components(&[id]);
                    }
                    let t = types[rng.below(types.len())];
                    let pos = [rng.below(40) as i32, rng.below(40) as i32];
                    e.place_component(t, pos).map(|_| ())
                }
                // Wire two random ports.
                20..=49 => {
                    let ports = all_ports(&e);
                    if ports.len() < 2 {
                        return Ok(());
                    }
                    let from = ports[rng.below(ports.len())].clone();
                    let to = ports[rng.below(ports.len())].clone();
                    e.add_wire(WirePoint::Port(from), WirePoint::Port(to))
                        .map(|_| ())
                }
                // Wire a port to a fresh node, or to an existing junction.
                50..=59 => {
                    let ports = all_ports(&e);
                    if ports.is_empty() {
                        return Ok(());
                    }
                    let from = ports[rng.below(ports.len())].clone();
                    let at = [rng.below(80) as f32 / 2.0, rng.below(80) as f32 / 2.0];
                    e.add_wire(WirePoint::Port(from), WirePoint::New(at))
                        .map(|_| ())
                }
                // Tee into a random segment.
                60..=64 => {
                    let (_, nets) = e.sheet_data();
                    let candidates: Vec<(String, usize)> = nets
                        .iter()
                        .flat_map(|n| {
                            (0..n.segments.len()).map(move |i| (n.id.clone(), i))
                        })
                        .collect();
                    if candidates.is_empty() {
                        return Ok(());
                    }
                    let (net, seg) = candidates[rng.below(candidates.len())].clone();
                    let ports = all_ports(&e);
                    if ports.is_empty() {
                        return Ok(());
                    }
                    let from = ports[rng.below(ports.len())].clone();
                    let at = [rng.below(80) as f32 / 2.0, rng.below(80) as f32 / 2.0];
                    e.add_wire_at_segment(WirePoint::Port(from), &net, seg, at)
                        .map(|_| ())
                }
                // Delete a random segment.
                65..=72 => {
                    let (_, nets) = e.sheet_data();
                    let candidates: Vec<(String, usize)> = nets
                        .iter()
                        .flat_map(|n| {
                            (0..n.segments.len()).map(move |i| (n.id.clone(), i))
                        })
                        .collect();
                    if candidates.is_empty() {
                        return Ok(());
                    }
                    let pick = candidates[rng.below(candidates.len())].clone();
                    e.delete_segments(&[pick])
                }
                // Delete a random component.
                73..=79 => {
                    let (components, _) = e.sheet_data();
                    if components.is_empty() {
                        return Ok(());
                    }
                    let id = components[rng.below(components.len())].id.clone();
                    e.delete_components(&[id])
                }
                // Fiddle with pins (on the abstraction sheet).
                80..=84 => {
                    let SheetRef::Abstraction(i) = e.sheet else {
                        return Ok(());
                    };
                    let def = &e.project.abstractions[i];
                    if def.inputs.len() + def.outputs.len() >= 12 {
                        return Ok(());
                    }
                    let name = format!("p{}", rng.below(1000));
                    let kind = if rng.chance(50) {
                        PortKind::Input
                    } else {
                        PortKind::Output
                    };
                    let width = if rng.chance(30) { Some(8) } else { None };
                    e.add_interface_pin(kind, &name, width)
                }
                85..=87 => {
                    if e.sheet == SheetRef::Top {
                        return Ok(());
                    }
                    let SheetRef::Abstraction(i) = e.sheet else {
                        return Ok(());
                    };
                    let def = &e.project.abstractions[i];
                    let mut pins: Vec<(PortKind, String)> = def
                        .inputs
                        .iter()
                        .map(|n| (PortKind::Input, n.clone()))
                        .collect();
                    pins.extend(def.outputs.keys().map(|n| (PortKind::Output, n.clone())));
                    if pins.is_empty() {
                        return Ok(());
                    }
                    let (kind, name) = pins[rng.below(pins.len())].clone();
                    e.remove_interface_pin(kind, &name)
                }
                // Change a bundler's width.
                88..=90 => {
                    let (components, _) = e.sheet_data();
                    let bundlers: Vec<String> = components
                        .iter()
                        .filter(|c| crate::layout::is_bundle_component(&c.component_type))
                        .map(|c| c.id.clone())
                        .collect();
                    if bundlers.is_empty() {
                        return Ok(());
                    }
                    let id = bundlers[rng.below(bundlers.len())].clone();
                    let w = 2 + rng.below(14) as u32;
                    e.set_bundle_width(&id, w)
                }
                // Copy/paste a random component (same crowding cap as place).
                91..=93 => {
                    let (components, _) = e.sheet_data();
                    if components.is_empty() || components.len() >= 30 {
                        return Ok(());
                    }
                    let id = components[rng.below(components.len())].id.clone();
                    let frag = e.fragment(&[id], &[], &[]);
                    e.paste_fragment(&frag, [1, 1], true).map(|_| ())
                }
                // Undo / redo.
                94..=96 => e.undo(),
                97 => e.redo(),
                // Switch sheets.
                _ => {
                    let target = if e.sheet == SheetRef::Top {
                        SheetRef::Abstraction(a)
                    } else {
                        SheetRef::Top
                    };
                    e.open_sheet(target)
                }
            }
        })();
        ok += outcome.is_ok() as usize;
        let ms = t0.elapsed().as_millis();
        if ms > 100 {
            let nets: usize = e.project.wire_nets.len()
                + e.project.abstractions.iter().map(|a| a.wire_nets.len()).sum::<usize>();
            let segs: usize = e.project.wire_nets.iter().map(|n| n.segments.len()).sum::<usize>()
                + e.project.abstractions.iter()
                    .flat_map(|a| &a.wire_nets).map(|n| n.segments.len()).sum::<usize>();
            let comps: usize = e.project.components.len()
                + e.project.abstractions.iter().map(|a| a.components.len()).sum::<usize>();
            eprintln!("SLOW seed {seed:x} step {step} roll {roll}: {ms}ms ({comps} comps, {nets} nets, {segs} segs)");
        }

        if let Err(msg) = check_invariants(&e.project) {
            panic!("seed {seed}, step {step} (roll {roll}): {msg}");
        }
    }
    // Guard against the battery degenerating into a refusal loop.
    assert!(
        ok > steps / 2,
        "seed {seed}: only {ok}/{steps} ops succeeded — the battery isn't exercising much"
    );

    // The end state must also survive a save/load round-trip cleanly.
    let json = serde_json::to_string(&e.project).unwrap();
    let mut reloaded: crate::project::Project = serde_json::from_str(&json).unwrap();
    let notes = normalize(&mut reloaded);
    assert!(
        notes.is_empty(),
        "seed {seed}: a settled document normalizes clean, but: {notes:?}"
    );
    if let Err(msg) = check_invariants(&reloaded) {
        panic!("seed {seed}: reloaded document inconsistent: {msg}");
    }
}

#[test]
fn stress_battery_seed_1() {
    run_battery(0x00C0FFEE, 800);
}

#[test]
fn stress_battery_seed_2() {
    run_battery(0xDEADBEEF, 800);
}

#[test]
fn stress_battery_seed_3() {
    run_battery(0x12345678, 800);
}

#[test]
fn stress_battery_seed_4() {
    run_battery(0xFEEDF00D, 800);
}
