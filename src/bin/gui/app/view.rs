//! The zoomable world: focus descent/ascent along the instance path,
//! camera rebasing, what the title bar says, and the document as it
//! should be DRAWN mid-gesture.


use diycpu::editor::SheetRef;
use diycpu::layout::{self, Rect};
use diycpu::project::{PortRef, Project};


use super::*;


impl App {
    pub fn focus_sheet(&self) -> SheetRef {
        self.path.last().map(|s| s.child).unwrap_or(self.view_root)
    }

    pub(super) fn viewport_min(&self) -> f32 {
        let r = self.canvas_rect();
        r.w.min(r.h)
    }

    pub(super) fn canvas_center(&self) -> [f32; 2] {
        let r = self.canvas_rect();
        [r.x + r.w / 2.0, r.y + r.h / 2.0]
    }

    /// The canvas rect inset by `frac` of its smaller dimension per side —
    /// "what the user is looking at".
    pub(super) fn central_box(&self, frac: f32) -> Rect {
        let r = self.canvas_rect();
        let inset = r.w.min(r.h) * frac;
        Rect {
            x: r.x + inset,
            y: r.y + inset,
            w: (r.w - inset * 2.0).max(1.0),
            h: (r.h - inset * 2.0).max(1.0),
        }
    }

    /// The child→parent transform of one zoom-path segment, recomputed from
    /// the given document state.
    pub fn seg_map_in(
        &self,
        project: &Project,
        seg: &PathSeg,
    ) -> Option<diycpu::layout::SheetMap> {
        let abstractions = &project.abstractions;
        let (components, _) = diycpu::editor::sheet_of(project, seg.parent)?;
        let comp = components.iter().find(|c| c.id == seg.instance_id)?;
        let def = abstractions.iter().find(|a| a.name == comp.component_type)?;
        Some(diycpu::layout::instance_inner_map(
            abstractions,
            def,
            layout::footprint(abstractions, comp),
        ))
    }

    pub fn seg_map(&self, seg: &PathSeg) -> Option<diycpu::layout::SheetMap> {
        self.seg_map_in(&self.editor.project, seg)
    }

    /// Keep the focused sheet in step with the camera: zooming into an
    /// instance until it fills the screen hands focus (and the camera's
    /// coordinate system) to its sheet; zooming back out hands it back.
    /// The rebased camera describes the identical visual transform, so the
    /// hand-off is invisible.
    pub fn update_focus(&mut self) {
        if !matches!(self.gesture, Gesture::Idle | Gesture::Panning { .. }) {
            return;
        }
        for _ in 0..16 {
            if !self.try_descend() && !self.try_ascend() {
                break;
            }
        }
    }

    pub(super) fn try_descend(&mut self) -> bool {
        let view = self.camera.view();
        let v = self.viewport_min();
        let abstractions = &self.editor.project.abstractions;
        let (components, _) = self.editor.sheet_data();

        let central = self.central_box(DESCEND_COVER_INSET);

        let mut target: Option<(String, usize, layout::Rect)> = None;
        for comp in components {
            let Some(def_index) = abstractions
                .iter()
                .position(|a| a.name == comp.component_type)
            else {
                continue;
            };
            let rect = layout::footprint(abstractions, comp);
            let tl = view.to_screen([rect.x, rect.y]);
            let (w, h) = (rect.w * view.scale, rect.h * view.scale);
            // "We're looking at it": the instance covers the whole central
            // region of the screen (components can't overlap, so at most
            // one instance can).
            let covers = tl[0] <= central.x
                && tl[1] <= central.y
                && tl[0] + w >= central.x + central.w
                && tl[1] + h >= central.y + central.h;
            if w.max(h) > DESCEND_AT * v && covers {
                target = Some((comp.id.clone(), def_index, rect));
                break;
            }
        }
        let Some((instance_id, def_index, rect)) = target else {
            return false;
        };

        let def = &self.editor.project.abstractions[def_index];
        let map = diycpu::layout::instance_inner_map(
            &self.editor.project.abstractions,
            def,
            rect,
        );
        self.camera.set_view(view.through(&map));
        self.path.push(PathSeg {
            parent: self.editor.sheet,
            instance_id,
            child: SheetRef::Abstraction(def_index),
        });
        let _ = self.editor.open_sheet(SheetRef::Abstraction(def_index));
        self.selection.clear();
        self.recompute_hover();
        true
    }

    pub(super) fn try_ascend(&mut self) -> bool {
        let Some(seg) = self.path.last().cloned() else {
            return false;
        };
        let Some(map) = self.seg_map(&seg) else {
            // The enclosing instance vanished (undo, delete): snap out.
            self.reset_view_to_editor_sheet();
            return false;
        };
        let parent_view = self.camera.view().through(&map.inverse());
        let abstractions = &self.editor.project.abstractions;
        let (parent_components, _) =
            diycpu::editor::sheet_of(&self.editor.project, seg.parent).unwrap_or((&[], &[]));
        let Some(comp) = parent_components.iter().find(|c| c.id == seg.instance_id) else {
            self.reset_view_to_editor_sheet();
            return false;
        };
        let rect = layout::footprint(abstractions, comp);
        let tl = parent_view.to_screen([rect.x, rect.y]);
        let (w, h) = (rect.w * parent_view.scale, rect.h * parent_view.scale);
        let v = self.viewport_min();
        let central = self.central_box(ASCEND_COVER_INSET);
        let covers = tl[0] <= central.x
            && tl[1] <= central.y
            && tl[0] + w >= central.x + central.w
            && tl[1] + h >= central.y + central.h;
        if w.max(h) >= ASCEND_AT * v && covers {
            return false;
        }
        self.camera.set_view(parent_view);
        self.path.pop();
        let _ = self.editor.open_sheet(seg.parent);
        self.selection.clear();
        self.recompute_hover();
        true
    }

    /// One explicit step outward (Esc / Back button), seamlessly rebased.
    pub(super) fn ascend_once(&mut self) -> bool {
        let Some(seg) = self.path.last().cloned() else {
            return false;
        };
        if let Some(map) = self.seg_map(&seg) {
            self.camera.set_view(self.camera.view().through(&map.inverse()));
        }
        self.path.pop();
        let _ = self.editor.open_sheet(seg.parent);
        self.selection.clear();
        self.gesture = Gesture::Idle;
        self.recompute_hover();
        true
    }

    /// The name of the abstraction a sheet ref points at, if any — used to
    /// notice when an index silently changes identity across undo/redo.
    pub(super) fn sheet_identity(&self, sheet: SheetRef) -> Option<String> {
        match sheet {
            SheetRef::Top => None,
            SheetRef::Abstraction(i) => {
                self.editor.project.abstractions.get(i).map(|a| a.name.clone())
            }
        }
    }

    /// The editor's sheet changed underneath the view (undo across sheets,
    /// abstraction deleted): re-root the view there.
    pub(super) fn reset_view_to_editor_sheet(&mut self) {
        self.view_root = self.editor.sheet;
        self.path.clear();
        self.pending_fit = true;
    }

    /// Open an abstraction as the root of the view (sidebar affordance).
    pub fn open_as_root(&mut self, index: usize) {
        if !self.require_edit_mode() {
            return;
        }
        match self.editor.open_sheet(SheetRef::Abstraction(index)) {
            Ok(()) => {
                self.view_root = SheetRef::Abstraction(index);
                self.path.clear();
                self.selection.clear();
                self.gesture = Gesture::Idle;
                self.pending_fit = true;
                self.info("Editing the abstraction — every instance updates; Esc goes back");
            }
            Err(e) => self.error(e.to_string()),
        }
    }

    /// The project as it should be DRAWN right now: mid-drag gestures are
    /// applied to a temporary copy so components render fully and wires
    /// re-route live, while the real document stays untouched until the
    /// gesture commits.
    pub fn display_project(&self) -> std::borrow::Cow<'_, Project> {
        use std::borrow::Cow;
        let sheet = self.editor.sheet;
        match &self.gesture {
            Gesture::DraggingComponents {
                ids,
                note_ids,
                image_ids,
                delta,
                copying,
                ..
            } if *delta != [0, 0] => {
                let mut p = self.editor.project.clone();
                let (components, notes, images) = match sheet {
                    SheetRef::Top => (&mut p.components, &mut p.notes, &mut p.images),
                    SheetRef::Abstraction(i) => {
                        let a = &mut p.abstractions[i];
                        (&mut a.components, &mut a.notes, &mut a.images)
                    }
                };
                if *copying {
                    // Preview phantoms at the target; the originals stay.
                    let phantoms: Vec<_> = components
                        .iter()
                        .filter(|c| ids.contains(&c.id))
                        .map(|c| {
                            let mut ghost = c.clone();
                            ghost.id = format!("__copy_{}", ghost.id);
                            ghost.pos[0] += delta[0] as f32;
                            ghost.pos[1] += delta[1] as f32;
                            ghost
                        })
                        .collect();
                    components.extend(phantoms);
                    let note_phantoms: Vec<_> = notes
                        .iter()
                        .filter(|n| note_ids.contains(&n.id))
                        .map(|n| {
                            let mut ghost = n.clone();
                            ghost.id = format!("__copy_{}", ghost.id);
                            ghost.pos[0] += delta[0] as f32;
                            ghost.pos[1] += delta[1] as f32;
                            ghost
                        })
                        .collect();
                    notes.extend(note_phantoms);
                    let image_phantoms: Vec<_> = images
                        .iter()
                        .filter(|i| image_ids.contains(&i.id))
                        .map(|i| {
                            let mut ghost = i.clone();
                            ghost.id = format!("__copy_{}", ghost.id);
                            ghost.pos[0] += delta[0] as f32;
                            ghost.pos[1] += delta[1] as f32;
                            ghost
                        })
                        .collect();
                    images.extend(image_phantoms);
                } else {
                    for c in components.iter_mut().filter(|c| ids.contains(&c.id)) {
                        c.pos[0] += delta[0] as f32;
                        c.pos[1] += delta[1] as f32;
                    }
                    for n in notes.iter_mut().filter(|n| note_ids.contains(&n.id)) {
                        n.pos[0] += delta[0] as f32;
                        n.pos[1] += delta[1] as f32;
                    }
                    for i in images.iter_mut().filter(|i| image_ids.contains(&i.id)) {
                        i.pos[0] += delta[0] as f32;
                        i.pos[1] += delta[1] as f32;
                    }
                }
                Cow::Owned(p)
            }
            Gesture::DraggingJunction {
                net_id,
                junction_id,
                pos,
            } => {
                let mut p = self.editor.project.clone();
                let wire_nets = match sheet {
                    SheetRef::Top => &mut p.wire_nets,
                    SheetRef::Abstraction(i) => &mut p.abstractions[i].wire_nets,
                };
                if let Some(j) = wire_nets
                    .iter_mut()
                    .find(|n| &n.id == net_id)
                    .and_then(|n| n.junctions.iter_mut().find(|j| &j.id == junction_id))
                {
                    j.pos = *pos;
                }
                Cow::Owned(p)
            }
            Gesture::DraggingPin { name, pos } => {
                let mut p = self.editor.project.clone();
                if let SheetRef::Abstraction(i) = sheet {
                    p.abstractions[i].pin_positions.insert(name.clone(), *pos);
                }
                Cow::Owned(p)
            }
            _ => Cow::Borrowed(&self.editor.project),
        }
    }

    /// The title-bar trail: project ▸ root abstraction ▸ … ▸ focus.
    pub fn breadcrumb(&self) -> Vec<String> {
        let abstractions = &self.editor.project.abstractions;
        let name_of = |sheet: &SheetRef| match sheet {
            SheetRef::Top => None,
            SheetRef::Abstraction(i) => abstractions.get(*i).map(|a| a.name.clone()),
        };
        let mut parts = vec![self.editor.project.name.clone()];
        parts.extend(name_of(&self.view_root));
        for seg in &self.path {
            parts.extend(name_of(&seg.child));
        }
        parts
    }

    pub fn content_bounds(&self) -> Option<Rect> {
        let abstractions = &self.editor.project.abstractions;
        let (components, _) = self.editor.sheet_data();
        let mut bounds = layout::components_bounds(abstractions, components);
        for img in self.editor.sheet_images() {
            let r = layout::image_bounds(img);
            bounds = Some(bounds.map_or(r, |b| b.union(&r)));
        }
        // Interface pins float beside the content — include them.
        if let Some(def) = self.interface() {
            for pin in layout::interface_pins(abstractions, def) {
                bounds = Some(bounds.map_or(pin.plate, |b| b.union(&pin.plate)));
            }
        }
        bounds
    }

    pub fn fit_view(&mut self) {
        let content = self.content_bounds().unwrap_or(Rect {
            x: 0.0,
            y: 0.0,
            w: 0.0,
            h: 0.0,
        });
        self.camera.fit(content, self.canvas_rect(), 60.0);
        self.recompute_hover();
    }

    pub fn action_back_to_top(&mut self) {
        if self.ascend_once() {
            self.info("Zoomed out one level");
        } else if self.view_root != SheetRef::Top {
            self.view_root = SheetRef::Top;
            let _ = self.editor.open_sheet(SheetRef::Top);
            self.selection.clear();
            self.gesture = Gesture::Idle;
            self.pending_fit = true;
            self.info("Back to the main circuit");
        }
    }
}

/// One level of the zoom path: which instance on `parent` we're inside.
#[derive(Clone, PartialEq)]
pub struct PathSeg {
    pub parent: SheetRef,
    pub instance_id: String,
    pub child: SheetRef,
}

/// Focus descends into an instance when it covers the whole central region
/// of the viewport (canvas inset by this fraction of its smaller dimension
/// per side) AND is comfortably larger than the screen; it ascends when the
/// enclosing instance stops covering a slightly smaller region. The gap is
/// hysteresis.
pub(crate) const DESCEND_COVER_INSET: f32 = 0.20;

pub(crate) const ASCEND_COVER_INSET: f32 = 0.24;

pub(crate) const DESCEND_AT: f32 = 1.25;

pub(crate) const ASCEND_AT: f32 = 1.10;

/// The true driver behind a port, for run-mode display: an unbundler's
/// lane output resolves through the sheet's bundle wiring to whatever
/// feeds that lane (chains included). Ordinary ports return themselves.
/// None means "no defined driver here" — render neutral.
pub fn resolve_lane_display(
    components: &[diycpu::project::ProjectComponent],
    wire_nets: &[diycpu::project::WireNet],
    port: &PortRef,
    depth: usize,
) -> Option<PortRef> {
    if depth > 16 {
        return None;
    }
    let comp = components.iter().find(|c| c.id == port.component_id)?;
    if comp.component_type != "unbundle" {
        return Some(port.clone());
    }
    let lane: u32 = port.port_name.parse().ok()?;
    let feed = wire_nets
        .iter()
        .filter(|n| n.bundle)
        .find(|n| {
            n.sinks
                .iter()
                .any(|s| s.component_id == comp.id && s.port_name == "bundle")
        })
        .and_then(|n| n.source.clone())?;
    resolve_bundle_lane(components, wire_nets, &feed, lane, depth + 1)
}

/// The driver of lane `k` behind a bundle SOURCE port (a bundler's output,
/// an instance's bundle output, or nothing resolvable). The other half of
/// [`resolve_lane_display`] — displays fed by bundles read through this.
pub fn resolve_bundle_lane(
    components: &[diycpu::project::ProjectComponent],
    wire_nets: &[diycpu::project::WireNet],
    src: &PortRef,
    k: u32,
    depth: usize,
) -> Option<PortRef> {
    if depth > 16 {
        return None;
    }
    let comp = components.iter().find(|c| c.id == src.component_id)?;
    if comp.component_type == "bundle" {
        let lane = k.to_string();
        let feed = wire_nets
            .iter()
            .filter(|n| !n.bundle)
            .find(|n| {
                n.sinks
                    .iter()
                    .any(|s| s.component_id == comp.id && s.port_name == lane)
            })
            .and_then(|n| n.source.clone())?;
        resolve_lane_display(components, wire_nets, &feed, depth + 1)
    } else {
        // An instance's bundle output: its per-lane engine port.
        Some(PortRef {
            component_id: src.component_id.clone(),
            port_name: format!("{}__b{k}", src.port_name),
        })
    }
}
