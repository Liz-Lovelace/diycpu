# Frontend architecture

The GUI is a layered editor over the exact same data the backend consumes.
There is no GUI-local mirror of the design: what you see is rendered from the
`Project` struct, and what you save is that same struct. This is the core
defense against the "canvas shows one thing, file contains another" class of
bugs.

## Layers

```
input events (winit)
        │
        ▼
┌─────────────────────┐     reads      ┌──────────────────────┐
│ interaction         │───────────────▶│ layout (pure)        │
│ gesture state       │                │ footprints, ports,   │
│ machine             │                │ hit-testing, routing │
└────────┬────────────┘                └──────────────────────┘
         │ commits intents (once, at gesture end)
         ▼
┌─────────────────────┐
│ editor              │  validated operations + snapshot undo
│ owns Project        │  place / move / connect / delete / …
└────────┬────────────┘
         │ document + derived geometry
         ▼
┌─────────────────────┐                ┌──────────────────────┐
│ render (pure)       │◀───────────────│ sim session          │
│ theme + drawing     │   wire values  │ compile + replay     │
└─────────────────────┘                └──────────────────────┘
```

### Document (`diycpu::project::Project`)
The single source of truth. Every persistent thing — component positions,
wire nets, junctions, segments, abstractions — lives here and only here.
Editing in the GUI and editing the JSON by hand are the same operation.

### Editor (`editor.rs`)
Owns the `Project` plus undo/redo stacks. All mutations go through
intent-level operations (`place_component`, `move_components`, `connect`,
`delete`, …). Each operation:

1. validates against the document invariants (below); invalid intents are
   rejected with a human-readable reason (shown in the status bar), leaving
   the document untouched;
2. mutates the document;
3. participates in undo via whole-`Project` snapshots.

Snapshot undo is deliberate: designs are toy-scale, and snapshotting the
whole document makes undo correct by construction — there is no operation
whose inverse can be implemented wrong. A drag is one undo step (snapshot
taken when the gesture commits, not per mouse-move).

Operations are generic over a **sheet** — either the top level
(`Project.components` / `Project.wire_nets`) or the inside of one
abstraction (`ProjectAbstraction.components` / `.wire_nets`). Abstraction
editing is the same editor pointed at a different sheet.

### Document invariants (enforced by every editor op)
- Component IDs are unique per sheet; generated as `nand_1`, `dff_2`, …
- A wire net has at most one source, and it is always an *output* port.
- An input port is a sink of at most one net across the whole sheet.
  Connecting to an occupied input replaces the old connection.
- At most one `clk` component per design (compiler rule, enforced at edit
  time rather than at compile time).
- Deleting a component removes every reference to it: nets it sources lose
  their source, nets it sinks drop that sink, segments touching its ports
  are removed, and nets left with no logical endpoints are deleted.
- Segments only reference junctions that exist in their net and ports that
  exist on the sheet.
- Components never overlap: moves/placements that would overlap another
  footprint are rejected (the drag preview shows invalid-red and the drop
  snaps back).

### Layout (`layout.rs`) — the "intermediate representation"
Pure functions from document to geometry; holds no state. Given a component
(primitive or abstraction instance) it yields the footprint in grid units and
the position/direction/kind of every port. Given a net it yields the routed
polyline of every segment. Hit-testing ("what is under this grid point:
port / body / wire / nothing") lives here too, so interaction logic never
does its own geometry math.

Everything is in **grid units** (one cell = 40 px at zoom 1). Component
positions snap to integer cells; ports sit on component edges at cell
centers. `pos`, and abstraction `width`/`height`, are stored in grid units.

### Wire model
The document stores logical connectivity (`source`/`sinks`) *and* visual
routing (`junctions`/`segments`), as before. The rules that keep them honest:

- Segment endpoints reference ports symbolically, so wires follow components
  automatically — there are no stored wire pixel-coordinates to go stale.
- There is exactly **one** routing algorithm (`layout::route_segment`): a
  deterministic orthogonal elbow between two points with port stub-outs. The
  committed wire, the live drag preview, and hit-testing all use it. The
  preview can therefore never differ from the wire you end up with.
- `connect(output, input)` merges into the existing net of that output (a
  net = one source, many sinks) and appends a port→port segment. Junctions
  are optional user-placed branch points, not an implicit side effect.

### Interaction (`app.rs`)
A single gesture enum — `Idle`, `Panning`, `PlacingComponent`,
`DraggingComponents`, `DrawingWire`, `BandSelecting` — so it is impossible
to be in two gestures at once. Gestures hold only ephemeral data (anchor
point, ghost position, pending port) and never touch the document until they
commit exactly once at mouse-up. `Esc` (or starting undo, entering run mode,
losing window focus) cancels the active gesture by simply dropping it — the
document was never touched, so there is nothing to roll back.

### Renderer (`render.rs`, `theme.rs`)
Pure: draws (document, gesture, sim values, camera) each frame. Light theme.
No retained scene graph, no cached positions.

### Simulation (`sim.rs`)
`Mode::Edit` / `Mode::Run`. Entering Run converts → expands → compiles the
project; errors land in the status bar and you stay in Edit. Run mode locks
editing and supports stepping the clock forward *and back* — states are
recomputed by replaying from step 0, which is trivially cheap at toy scale
and keeps the session stateless. A wire is energized iff its net's source
value is true in the current step (abstraction outputs resolve through the
expander's aliases).

## Testing
The editor and layout layers are pure Rust with no GL dependency and carry
the edge-case burden: invariant enforcement, connect/delete/undo interactions,
hit-testing. The renderer is verified visually via `--screenshot` (headless
Xvfb render to PNG).
