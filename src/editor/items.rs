//! Operations on the things that sit on a sheet: components, notes
//! and images — placing, moving, deleting, and their small settings.



use crate::layout::{self, Rect};
use crate::project::{Endpoint, ProjectComponent};

use super::*;


impl Editor {
    /// Place a new component (primitive or abstraction instance) with its
    /// top-left corner at integer grid position `pos`.
    pub fn place_component(
        &mut self,
        type_name: &str,
        pos: [i32; 2],
    ) -> Result<String, EditError> {
        let type_name = type_name.to_string();
        self.mutate(move |project, sheet| {
            let size = layout::type_size(&project.abstractions, &type_name)
                .ok_or_else(|| EditError::UnknownComponentType(type_name.clone()))?;

            // An abstraction may not (transitively) contain itself.
            if let SheetRef::Abstraction(i) = sheet {
                let host = project
                    .abstractions
                    .get(i)
                    .ok_or(EditError::BadSheet)?
                    .name
                    .clone();
                if abstraction_uses(&project.abstractions, &type_name, &host) {
                    return Err(EditError::RecursiveAbstraction(type_name.clone()));
                }
            }

            let rect = Rect {
                x: pos[0] as f32,
                y: pos[1] as f32,
                w: size[0],
                h: size[1],
            };
            let abstractions = project.abstractions.clone();
            let (components, _) = sheet_mut(project, sheet).ok_or(EditError::BadSheet)?;
            for existing in components.iter() {
                if layout::footprint(&abstractions, existing).intersects(&rect) {
                    return Err(EditError::Overlap);
                }
            }

            let id = generate_id(components, &type_name);
            components.push(ProjectComponent {
                display: None,
                bits: None,
                id: id.clone(),
                component_type: type_name.clone(),
                init: (type_name == "dff" || type_name == "const").then_some(false),
                pos: [pos[0] as f32, pos[1] as f32],
            });
            Ok(id)
        })
    }

    /// Move a set of components by a whole number of grid cells. Rejected if
    /// any moved footprint would overlap a component outside the set.
    pub fn move_components(
        &mut self,
        ids: &[String],
        delta: [i32; 2],
    ) -> Result<(), EditError> {
        self.move_items(ids, &[], &[], delta)
    }

    /// Move components, notes and images together by a whole number of grid
    /// cells, as one undo step. Only components have overlap rules.
    pub fn move_items(
        &mut self,
        component_ids: &[String],
        note_ids: &[String],
        image_ids: &[String],
        delta: [i32; 2],
    ) -> Result<(), EditError> {
        if delta == [0, 0] {
            return Ok(()); // not an edit; don't spend an undo step
        }
        let ids = component_ids.to_vec();
        let note_ids = note_ids.to_vec();
        let image_ids = image_ids.to_vec();
        self.mutate(move |project, sheet| {
            let abstractions = project.abstractions.clone();
            let (components, _) = sheet_mut(project, sheet).ok_or(EditError::BadSheet)?;

            let mut moved_rects = Vec::new();
            for id in &ids {
                let comp = components
                    .iter()
                    .find(|c| &c.id == id)
                    .ok_or_else(|| EditError::UnknownComponent(id.clone()))?;
                let mut rect = layout::footprint(&abstractions, comp);
                rect.x += delta[0] as f32;
                rect.y += delta[1] as f32;
                moved_rects.push(rect);
            }
            for other in components.iter().filter(|c| !ids.contains(&c.id)) {
                let other_rect = layout::footprint(&abstractions, other);
                if moved_rects.iter().any(|r| r.intersects(&other_rect)) {
                    return Err(EditError::Overlap);
                }
            }

            for comp in components.iter_mut().filter(|c| ids.contains(&c.id)) {
                comp.pos[0] += delta[0] as f32;
                comp.pos[1] += delta[1] as f32;
            }

            let notes = sheet_notes_mut(project, sheet).ok_or(EditError::BadSheet)?;
            for id in &note_ids {
                let note = notes
                    .iter_mut()
                    .find(|n| &n.id == id)
                    .ok_or_else(|| EditError::UnknownNote(id.clone()))?;
                note.pos[0] += delta[0] as f32;
                note.pos[1] += delta[1] as f32;
            }

            let images = sheet_images_mut(project, sheet).ok_or(EditError::BadSheet)?;
            for id in &image_ids {
                let img = images
                    .iter_mut()
                    .find(|i| &i.id == id)
                    .ok_or_else(|| EditError::UnknownImage(id.clone()))?;
                img.pos[0] += delta[0] as f32;
                img.pos[1] += delta[1] as f32;
            }
            Ok(())
        })
    }

    /// Delete components and scrub every reference to them from wire nets.
    pub fn delete_components(&mut self, ids: &[String]) -> Result<(), EditError> {
        self.delete_items(ids, &[], &[])
    }

    /// Delete components, notes and images as one undo step.
    pub fn delete_items(
        &mut self,
        component_ids: &[String],
        note_ids: &[String],
        image_ids: &[String],
    ) -> Result<(), EditError> {
        self.delete_mixed(component_ids, note_ids, image_ids, &[], &[])
    }

    /// Delete any mix of selected things — components, notes, images, wire
    /// segments — and unwire output pins, all in one undo step.
    pub fn delete_mixed(
        &mut self,
        component_ids: &[String],
        note_ids: &[String],
        image_ids: &[String],
        segments: &[(String, usize)],
        pin_wires: &[String],
    ) -> Result<(), EditError> {
        if component_ids.is_empty()
            && note_ids.is_empty()
            && image_ids.is_empty()
            && segments.is_empty()
            && pin_wires.is_empty()
        {
            return Ok(());
        }
        let ids = component_ids.to_vec();
        let note_ids = note_ids.to_vec();
        let image_ids = image_ids.to_vec();
        let segments = segments.to_vec();
        let pin_wires = pin_wires.to_vec();
        self.mutate(move |project, sheet| {
            let ctx = PortResolver::new(project, sheet)?;
            let (components, wire_nets) =
                sheet_mut(project, sheet).ok_or(EditError::BadSheet)?;
            for id in &ids {
                if !components.iter().any(|c| &c.id == id) {
                    return Err(EditError::UnknownComponent(id.clone()));
                }
            }
            components.retain(|c| !ids.contains(&c.id));

            // Explicitly selected segments go first (indices are valid
            // against the untouched nets), then everything referencing a
            // deleted component.
            for net in wire_nets.iter_mut() {
                let mut indices: Vec<usize> = segments
                    .iter()
                    .filter(|(id, _)| *id == net.id)
                    .map(|(_, i)| *i)
                    .collect();
                indices.sort_unstable();
                indices.dedup();
                for i in indices.into_iter().rev() {
                    if i < net.segments.len() {
                        net.segments.remove(i);
                    }
                }
            }
            for net in wire_nets.iter_mut() {
                net.segments.retain(|seg| {
                    ![&seg.from, &seg.to].iter().any(|e| match e {
                        Endpoint::Port { component_id, .. } => ids.contains(component_id),
                        Endpoint::Junction { .. } => false,
                    })
                });
            }
            repair_nets(wire_nets, components, &ctx)?;

            if !pin_wires.is_empty() {
                let SheetRef::Abstraction(i) = sheet else {
                    return Err(EditError::NotAnAbstractionSheet);
                };
                let def = project.abstractions.get_mut(i).ok_or(EditError::BadSheet)?;
                for name in &pin_wires {
                    if let Some(target) = def.outputs.get_mut(name) {
                        target.clear();
                    }
                }
            }

            // Output pins that pointed at a deleted component become unwired.
            if let SheetRef::Abstraction(i) = sheet {
                let def = project.abstractions.get_mut(i).ok_or(EditError::BadSheet)?;
                for target in def.outputs.values_mut() {
                    if !target.is_empty()
                        && ids.contains(&layout::output_target_port(target).component_id)
                    {
                        target.clear();
                    }
                }
            }

            let notes = sheet_notes_mut(project, sheet).ok_or(EditError::BadSheet)?;
            notes.retain(|n| !note_ids.contains(&n.id));
            let images = sheet_images_mut(project, sheet).ok_or(EditError::BadSheet)?;
            images.retain(|i| !image_ids.contains(&i.id));
            Ok(())
        })
    }

    /// Place a text annotation. Returns its id.
    pub fn place_note(&mut self, pos: [f32; 2], text: &str) -> Result<String, EditError> {
        let text = text.to_string();
        self.mutate(move |project, sheet| {
            let notes = sheet_notes_mut(project, sheet).ok_or(EditError::BadSheet)?;
            let id = generate_note_id(notes);
            notes.push(crate::project::Note {
                id: id.clone(),
                text: text.clone(),
                pos,
            });
            Ok(id)
        })
    }

    /// Change a note's text. Empty text deletes the note.
    pub fn set_note_text(&mut self, id: &str, text: &str) -> Result<(), EditError> {
        let id = id.to_string();
        let text = text.trim().to_string();
        self.mutate(move |project, sheet| {
            let notes = sheet_notes_mut(project, sheet).ok_or(EditError::BadSheet)?;
            if !notes.iter().any(|n| n.id == id) {
                return Err(EditError::UnknownNote(id.clone()));
            }
            if text.is_empty() {
                notes.retain(|n| n.id != id);
            } else {
                notes.iter_mut().find(|n| n.id == id).unwrap().text = text.clone();
            }
            Ok(())
        })
    }

    /// Place a picture. `file` is a project-dir-relative path (normally
    /// under "media/"). Returns the new image's id.
    pub fn place_image(
        &mut self,
        pos: [f32; 2],
        file: &str,
        size: [f32; 2],
    ) -> Result<String, EditError> {
        let file = file.to_string();
        self.mutate(move |project, sheet| {
            let images = sheet_images_mut(project, sheet).ok_or(EditError::BadSheet)?;
            let id = generate_image_id(images);
            images.push(crate::project::MediaImage {
                id: id.clone(),
                file: file.clone(),
                pos,
                size: clamp_image_size(size),
            });
            Ok(id)
        })
    }

    /// Swap a picture's file, optionally updating its display size in the
    /// same undo step (a new file usually brings a new aspect ratio).
    pub fn set_image_file(
        &mut self,
        id: &str,
        file: &str,
        size: Option<[f32; 2]>,
    ) -> Result<(), EditError> {
        let id = id.to_string();
        let file = file.to_string();
        self.mutate(move |project, sheet| {
            let images = sheet_images_mut(project, sheet).ok_or(EditError::BadSheet)?;
            let img = images
                .iter_mut()
                .find(|i| i.id == id)
                .ok_or_else(|| EditError::UnknownImage(id.clone()))?;
            img.file = file.clone();
            if let Some(size) = size {
                img.size = clamp_image_size(size);
            }
            Ok(())
        })
    }

    /// Resize a picture (grid units).
    pub fn set_image_size(&mut self, id: &str, w: f32, h: f32) -> Result<(), EditError> {
        let id = id.to_string();
        self.mutate(move |project, sheet| {
            let images = sheet_images_mut(project, sheet).ok_or(EditError::BadSheet)?;
            let img = images
                .iter_mut()
                .find(|i| i.id == id)
                .ok_or_else(|| EditError::UnknownImage(id.clone()))?;
            img.size = clamp_image_size([w, h]);
            Ok(())
        })
    }

    /// Configure a display element's look — and, for segment displays,
    /// the 8/16 type. One undo step. Resizes respect neighbours; a type
    /// change that leaves a mismatched bundle wire is kept and flashes.
    pub fn configure_display(
        &mut self,
        id: &str,
        config: crate::project::DisplayConfig,
        seg_bits: Option<u32>,
    ) -> Result<(), EditError> {
        let id = id.to_string();
        self.mutate(move |project, sheet| {
            let abstractions = project.abstractions.clone();
            let (components, _) = sheet_mut(project, sheet).ok_or(EditError::BadSheet)?;
            let index = components
                .iter()
                .position(|c| c.id == id)
                .ok_or_else(|| EditError::UnknownComponent(id.clone()))?;
            if !layout::is_display(&components[index].component_type) {
                return Err(EditError::UnknownComponent(id.clone()));
            }
            if let Some(bits) = seg_bits {
                if bits != 8 && bits != 16 {
                    return Err(EditError::BadBundleWidth);
                }
                components[index].bits = Some(bits);
            }
            let mut config = config;
            if let Some(size) = &mut config.size {
                size[0] = size[0].clamp(1.0, 40.0);
                size[1] = size[1].clamp(1.0, 40.0);
            }
            if let Some(scale) = &mut config.scale {
                *scale = scale.clamp(2.0, 40.0);
            }
            components[index].display = Some(config);

            // The new footprint must still fit where it stands.
            let rect = layout::footprint(&abstractions, &components[index]);
            for (k, other) in components.iter().enumerate() {
                if k != index
                    && layout::footprint(&abstractions, other).intersects(&rect)
                {
                    return Err(EditError::Overlap);
                }
            }
            Ok(())
        })
    }

    /// Flip a flip-flop's initial value.
    pub fn toggle_dff_init(&mut self, id: &str) -> Result<bool, EditError> {
        let id = id.to_string();
        self.mutate(move |project, sheet| {
            let (components, _) = sheet_mut(project, sheet).ok_or(EditError::BadSheet)?;
            let comp = components
                .iter_mut()
                .find(|c| c.id == id)
                .ok_or_else(|| EditError::UnknownComponent(id.clone()))?;
            if comp.component_type != "dff" && comp.component_type != "const" {
                return Err(EditError::NotADff(id.clone()));
            }
            let new = !comp.init.unwrap_or(false);
            comp.init = Some(new);
            Ok(new)
        })
    }
}

/// Pictures may be any size from half a cell to a whole wall.
pub(crate) fn clamp_image_size(size: [f32; 2]) -> [f32; 2] {
    let fix = |v: f32, fallback: f32| if v.is_finite() { v.clamp(0.5, 60.0) } else { fallback };
    [fix(size[0], 6.0), fix(size[1], 4.0)]
}
