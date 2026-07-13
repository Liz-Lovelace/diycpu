//! Copyable sheet fragments: what Ctrl+C carries and Ctrl+V plants.


use std::collections::HashMap;

use crate::layout::{self, Rect};
use crate::project::{Endpoint, ProjectComponent, WireNet};

use super::*;


impl Editor {
    /// A self-contained snippet of a sheet: components, notes, images and
    /// the wiring that runs among them. What Ctrl+C carries around.
    pub fn fragment(
        &self,
        component_ids: &[String],
        note_ids: &[String],
        image_ids: &[String],
    ) -> Fragment {
        let (components, wire_nets) = self.sheet_data();
        let components: Vec<ProjectComponent> = components
            .iter()
            .filter(|c| component_ids.contains(&c.id))
            .cloned()
            .collect();
        let notes: Vec<crate::project::Note> = self
            .sheet_notes()
            .iter()
            .filter(|n| note_ids.contains(&n.id))
            .cloned()
            .collect();
        let images: Vec<crate::project::MediaImage> = self
            .sheet_images()
            .iter()
            .filter(|i| image_ids.contains(&i.id))
            .cloned()
            .collect();

        // Keep only wiring fully inside the copied set: segments whose port
        // endpoints all reference copied components. Interface pins are NOT
        // part of the selection, so wires to them are cut like any other
        // outside connection (junction endpoints come along when used).
        let mut nets = Vec::new();
        for net in wire_nets {
            let mut kept = net.clone();
            kept.segments.retain(|seg| {
                [&seg.from, &seg.to].iter().all(|e| match e {
                    Endpoint::Port { component_id, .. } => {
                        component_ids.contains(component_id)
                    }
                    Endpoint::Junction { .. } => true,
                })
            });
            drop_orphan_junctions(&mut kept);
            if !kept.segments.is_empty() {
                nets.push(kept);
            }
        }
        Fragment {
            components,
            notes,
            images,
            wire_nets: nets,
        }
    }

    /// Insert a fragment's copies, offset by `delta` grid cells, with fresh
    /// ids everywhere. With `search`, nearby offsets are tried until the
    /// copies fit. Returns the new (component ids, note ids, image ids).
    pub fn paste_fragment(
        &mut self,
        fragment: &Fragment,
        delta: [i32; 2],
        search: bool,
    ) -> Result<PastedIds, EditError> {
        let fragment = fragment.clone();
        self.mutate(move |project, sheet| {
            let ctx = PortResolver::new(project, sheet)?;
            let abstractions = project.abstractions.clone();

            // Types must exist here, and pasting an abstraction into its
            // own (or a using) sheet would recurse.
            for comp in &fragment.components {
                if layout::type_ports(&abstractions, &comp.component_type).is_none() {
                    return Err(EditError::UnknownComponentType(
                        comp.component_type.clone(),
                    ));
                }
                if let SheetRef::Abstraction(i) = sheet {
                    let host = abstractions.get(i).ok_or(EditError::BadSheet)?.name.clone();
                    if abstraction_uses(&abstractions, &comp.component_type, &host) {
                        return Err(EditError::RecursiveAbstraction(
                            comp.component_type.clone(),
                        ));
                    }
                }
            }

            // Find an offset where every copy fits.
            let (components, _) = sheet_mut(project, sheet).ok_or(EditError::BadSheet)?;
            let fits = |components: &[ProjectComponent], d: [f32; 2]| -> bool {
                fragment.components.iter().all(|c| {
                    let mut r = layout::footprint(&abstractions, c);
                    r.x += d[0];
                    r.y += d[1];
                    components
                        .iter()
                        .all(|other| !layout::footprint(&abstractions, other).intersects(&r))
                })
            };
            let base = [delta[0] as f32, delta[1] as f32];
            let mut offset = None;
            if fits(components, base) {
                offset = Some(base);
            } else if search {
                'out: for ring in 1..60 {
                    let s = ring as f32;
                    for cand in [
                        [base[0] + s, base[1] + s],
                        [base[0] + s, base[1]],
                        [base[0], base[1] + s],
                        [base[0] - s, base[1]],
                        [base[0], base[1] - s],
                    ] {
                        if fits(components, cand) {
                            offset = Some(cand);
                            break 'out;
                        }
                    }
                }
            }
            let offset = offset.ok_or(EditError::Overlap)?;

            // Fresh component ids, remembering the mapping for the wires.
            let mut id_map: HashMap<String, String> = HashMap::new();
            let mut new_component_ids = Vec::new();
            for original in &fragment.components {
                let id = generate_id(components, &original.component_type);
                id_map.insert(original.id.clone(), id.clone());
                components.push(ProjectComponent {
                    bits: None,
                    id: id.clone(),
                    component_type: original.component_type.clone(),
                    init: original.init,
                    pos: [original.pos[0] + offset[0], original.pos[1] + offset[1]],
                });
                new_component_ids.push(id);
            }

            // Wires: fresh net + junction ids, ports remapped, everything
            // shifted; logical fields re-derived by the repair pass.
            let (components, wire_nets) =
                sheet_mut(project, sheet).ok_or(EditError::BadSheet)?;
            for net in &fragment.wire_nets {
                let mut copy = net.clone();
                if copy.segments.is_empty() {
                    continue;
                }
                copy.id = generate_net_id(wire_nets);
                copy.source = None;
                copy.sinks.clear();
                for junction in &mut copy.junctions {
                    junction.pos[0] += offset[0];
                    junction.pos[1] += offset[1];
                }
                for seg in &mut copy.segments {
                    for e in [&mut seg.from, &mut seg.to] {
                        if let Endpoint::Port { component_id, .. } = e {
                            if let Some(new_id) = id_map.get(component_id) {
                                *component_id = new_id.clone();
                            }
                        }
                    }
                }
                wire_nets.push(copy);
            }
            repair_nets(wire_nets, components, &ctx)?;

            let notes = sheet_notes_mut(project, sheet).ok_or(EditError::BadSheet)?;
            let mut new_note_ids = Vec::new();
            for original in &fragment.notes {
                let id = generate_note_id(notes);
                notes.push(crate::project::Note {
                    id: id.clone(),
                    text: original.text.clone(),
                    pos: [original.pos[0] + offset[0], original.pos[1] + offset[1]],
                });
                new_note_ids.push(id);
            }

            let images = sheet_images_mut(project, sheet).ok_or(EditError::BadSheet)?;
            let mut new_image_ids = Vec::new();
            for original in &fragment.images {
                let id = generate_image_id(images);
                images.push(crate::project::MediaImage {
                    id: id.clone(),
                    file: original.file.clone(),
                    pos: [original.pos[0] + offset[0], original.pos[1] + offset[1]],
                    size: original.size,
                });
                new_image_ids.push(id);
            }

            Ok((new_component_ids, new_note_ids, new_image_ids))
        })
    }
}

/// New ids after a paste: (components, notes, images).
pub type PastedIds = (Vec<String>, Vec<String>, Vec<String>);

/// A copyable snippet of a sheet.
#[derive(Debug, Clone)]
pub struct Fragment {
    pub components: Vec<ProjectComponent>,
    pub notes: Vec<crate::project::Note>,
    pub images: Vec<crate::project::MediaImage>,
    pub wire_nets: Vec<WireNet>,
}

impl Fragment {
    pub fn is_empty(&self) -> bool {
        self.components.is_empty() && self.notes.is_empty() && self.images.is_empty()
    }

    /// Bounding box of the fragment's contents, in grid units.
    pub fn bounds(&self, abstractions: &[crate::project::ProjectAbstraction]) -> Option<Rect> {
        let mut bounds: Option<Rect> = None;
        let mut extend = |r: Rect| {
            bounds = Some(bounds.map_or(r, |b| b.union(&r)));
        };
        for c in &self.components {
            extend(layout::footprint(abstractions, c));
        }
        for n in &self.notes {
            extend(layout::note_bounds(n));
        }
        for i in &self.images {
            extend(layout::image_bounds(i));
        }
        bounds
    }
}
