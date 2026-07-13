# Architecture

The system is a stack of layers. Each layer only looks *down*, and every
layer states its one rule in its module docs. When adding a feature, find
the layer each piece belongs to — if a change wants to smear across
layers, that's the design telling you to stop and split it.

```
┌──────────────────────────────────────────────────────────────┐
│ shell        src/bin/gui/main.rs — event loop, timers, CLI   │
├──────────────────────────────────────────────────────────────┤
│ render/      pure draw: (document, app state, camera) → px   │
│ ui.rs        chrome rects shared by renderer and hit-testing │
├──────────────────────────────────────────────────────────────┤
│ app/         gestures, selection, dialogs, files, view focus │
│ camera.rs    grid ↔ screen transform                         │
│ sim.rs       run-mode session (replay-based stepping)        │
├──────────────────────────────────────────────────────────────┤
│ editor/      validated, atomic, undoable ops on the document │
│ layout.rs    pure geometry IR: footprints, ports, hit tests  │
├──────────────────────────────────────────────────────────────┤
│ project.rs   the document: serde JSON, the single truth      │
├──────────────────────────────────────────────────────────────┤
│ engine       convert → expander → compiler → simulator       │
│              (bundles and GUI concepts never reach it)       │
└──────────────────────────────────────────────────────────────┘
```

## The rules that hold it together

- **The document is the only truth.** `project.rs` is what's on disk;
  wire segments are the truth of a net (`source`/`sinks` are derived);
  everything visual is recomputed from the document every frame.
- **All mutation goes through `Editor::mutate`**: snapshot before,
  rollback on error, one op = one undo step. An operation can never
  leave the document half-mutated, so the GUI can fire ops speculatively
  and just show the error.
- **Gestures never touch the document until they commit.** A gesture is
  ephemeral state in `app/gestures.rs`; dropping it is always safe.
  Mid-drag rendering uses `App::display_project()` — a temporary copy
  with the gesture applied.
- **The renderer only reads.** `render/` is a pure function; its
  geometry vocabulary is `SheetCtx::px/px_rect`, its simulation
  vocabulary is `SheetCtx::{sim_value, stored_bit, face_bit,
  output_value}` — the *only* places drawing asks the simulation
  anything.
- **The engine never learns GUI concepts.** `convert.rs` lowers const
  (→ input-less DFF) and the whole bundle layer (→ per-lane wires)
  before the expander runs. If a feature needs engine support, lower it
  in convert instead.
- **Frontend format additions are serde-default** so the engine and old
  files ignore them (`notes`, `images`, `bundle`, `bits`,
  `bundle_pins`, `port_layout`, `pin_positions`).

## Where things live

- `src/editor/` — one file per op family; `mod.rs` has the map.
  `sheets.rs` owns `PortResolver` (what is this port? — direction and
  signal/bundle kind, pseudo-pins included), which every validating op
  captures before mutating.
- `src/layout.rs` — geometry only, in **grid units**; stateless; no
  document mutation. Half-grid port slots are what make straight wires
  line up across components.
- `src/bin/gui/app/` — `mod.rs` has the map. The infinite zoom lives in
  `view.rs`: the camera is always in the focused sheet's coordinates,
  and descending/ascending re-expresses it through `SheetMap`s so
  hand-offs are visually invisible and floats stay bounded at any depth.
- `src/bin/gui/render/` — `mod.rs` has the map, the paint helpers and
  `SheetCtx`. `parts.rs` shows the pattern the rest of the renderer
  follows: measure once into a shape struct, then one small draw
  function per conceptual element.
- Tests live next to their subject (`editor/*tests*.rs` and the
  `app/*_tests.rs` files); the gesture tests drive the real state
  machine through a `Driver` that moves a fake cursor.

## Verification

- `cargo test --lib` (152) + `cargo test --features gui --bin
  diycpu-gui` (47); clippy is kept at zero warnings.
- Headless visuals: `Xvfb :99 &`, then `diycpu-gui file.json
  --screenshot out.png` with `--run N`, `--sheet name`, `--camera
  z,x,y`, `--cursor gx,gy`, `--dialog`, `--image-dialog`, `--anim-t s`.
  Refactors are checked by pixel-diffing screenshots before and after.
- femtovg gotcha: `set_size(w, h, 1.0)` always — we draw in physical
  pixels, and a real dpi would double-scale glyphs into blur.
