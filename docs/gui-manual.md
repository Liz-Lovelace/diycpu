# GUI manual

```
cargo run --features gui --bin diycpu-gui -- [project-dir | project.json]
```

Projects are **folders** — `<name>/project.json` plus a `media/` directory
for pictures — and they all live together in `~/.local/share/diycpu`
(the XDG data directory; `$XDG_DATA_HOME` is respected). Started with no argument, the app opens whichever project you
touched last; on the very first launch it creates a little “welcome”
blinker for you. Everything autosaves on every change.

Extra flags for headless work: `--screenshot out.png` (render one frame and
exit), `--run N` (enter run mode and step N times first), `--sheet name`
(open an abstraction's sheet), `--size WxH`, `--cursor gx,gy` (hover a grid
point), `--image-dialog`, `--anim-t seconds`.

## Editing

| Action | How |
|---|---|
| Place a component | Click it in the palette, then click the canvas. Shift-click the canvas to place several. |
| Move | Drag a component. Multi-selections move together. |
| Select | Click a component; Ctrl-click to add/remove; drag on empty canvas for a box select — anything the box touches is selected, wires included. |
| Wire | Click or drag from any port. Click a compatible port to connect; click empty canvas to drop a **node** and keep drawing from it; right-click or Esc to stop (everything placed stays). Wires are straight lines (diagonals welcome) — route them your way by placing nodes. Wiring an already-connected input replaces its old connection. A wire that would loop a signal combinationally back into itself (no flip-flop in the loop) is refused on the spot instead of failing later at Run. |
| Wire nodes | Click a node to draw a new branch from it (fan-out). Drag a node to move it. Drag anywhere on a wire to tear a new node out of it and place it in the same motion. Stubs are fine: wires don't need to connect anything to exist. |
| Wire selection | Wires select like components: click a segment (Ctrl-click adds more), box-select sweeps them up alongside parts, Delete removes exactly what's selected — the wire splits where it was cut. The wire feeding an output pin selects and deletes the same way. |
| Delete | Select, then Delete/Backspace. Deleting a component cleans up its connections but leaves your routed wires as stubs. |
| Flip a DFF's start value | Double-click it — the big digit on its face is the stored bit. |
| Constant signal | The “const” primitive is a toggleable 0/1 source (double-click flips it) — handy for feeding test inputs. Unconnected inputs always read as 0. |
| Bundles | The “bundle” component gathers its numbered inputs into one **bundle port**; “unbundle” splits one back out. From a bundle port you draw a **bundle wire** — routed exactly like any wire (nodes, branches, box-select), drawn as a wide rail wearing its lane count at each end — but bundles and single wires never mix, and two bundles only *connect* when their lane counts match. The cog on a bundler sets its lane count (2–32; there's no bundle of one). Lane k in is lane k out. Abstraction pins carry bundles too: name a pin `data:8` and its port becomes a square 8-line bundle port. The engine never sees any of this — bundles lower to their individual lanes. |
| Width mismatches | Changing a lane count under an existing bundle wire (a bundler's cog, a pin's `:n`, a display's type) never disconnects it: the wire stays, lanes matched from bit 0 — missing lanes read 0, extra lanes go nowhere — and it **flashes red** with its end numbers disagreeing until you fix one side. |
| Displays | The DISPLAYS palette section is for circuits you can look at: **light** (one wire; off when 0 or unconnected), **rgb** (a 3-line bundle mixing red/green/blue), **seg** (an 8- or 16-segment glyph on a dark slab, lane per segment), and **face** (a 10-segment face that lights up in its artwork colors — every color is one lane, both eyes' matching arcs light together). The cog opens their settings: shape or segment type, lit color, and size (lights take width×height; seg and face keep their aspect and take one size). Displays are pure sinks — the engine never sees them. |
| Notes | Pick “note” in the palette, click the canvas, type. Double-click a note to edit it (empty text removes it); drag to move; notes can sit anywhere. |
| Images | Pick “image” in the palette, choose a file (png/jpeg/gif/webp, up to 20 MB — the file is copied into the project's `media/` folder), click to place. Animated GIFs play right on the canvas. Images select, drag, box-select, copy and delete like everything else; the cog on a hovered image opens its settings (swap the file, resize with sliders). Video files (webm) aren't supported. |
| The cog ⚙ | Hover anything configurable and a little gear appears in its corner — that's where its settings live. On an abstraction instance it opens the layout dialog, on an image the image settings, on a note the text editor, and on a dff/const it flips the start value. |
| Copy / cut / paste | Ctrl+C / Ctrl+X / Ctrl+V — the selection travels with the wiring that runs among the copied parts; paste lands centered on your cursor. Ctrl+D duplicates in place. |
| Copy by dragging | Hold Ctrl and drag a selection: the originals stay, copies (wires included) land where you drop. Ctrl+click still toggles items in and out of the selection. |
| Undo / redo | Ctrl+Z / Ctrl+Shift+Z (or Ctrl+Y). One action = one undo step; a cancelled gesture never becomes one. |
| Cancel anything | Esc or right-click. |
| Pan / zoom | Middle- or right-drag; scroll wheel zooms at the cursor. `F` fits the circuit, Ctrl+0 resets zoom. |
| Files | Saving is automatic — every change writes straight through to the project folder. Ctrl+N names a new project, Ctrl+Shift+S (“Save copy”) forks the current one under a new name (media included), Ctrl+O opens another project folder. |
| Rename the project | Click its name in the title bar. |

## Infinite zoom

The canvas is one continuous zoomable world. Zoom toward any abstraction
instance and, as it approaches screen size, its label fades out and its
contents fade in — keep zooming and you're *inside*, editing its sheet,
with the parent circuit still visible around the frame. Zoom back out and
you're seamlessly back in the parent. This nests to any depth. Double-click
an instance to jump straight inside; Esc (or “◂ Back”) steps back out one
level. The title bar shows where you are (`project ▸ alu ▸ adder`).

## Abstractions

- **Create**: the “+ abstraction” button in the toolbar, then name it.
- **Edit**: zoom or double-click into any instance, or click the ↗ on the
  abstraction's sidebar row to open it as the root of the view.
- **Pins** are free-floating objects inside the sheet: “+ input” /
  “+ output” create them, and you drag them anywhere you like. Wire from
  an input pin into the circuit like any port; wire a component output
  into an output pin to define what it emits. Wiring an input pin
  **straight into an output pin** is fine — the output simply forwards
  that input (handy for splitters and plain feed-throughs). A pin can't
  be dropped on top of a port, node or wire: that would only look
  connected, so the drop is refused. The vertical order of the
  pins inside is the port order on the closed rectangle outside.
  Double-click a pin's name plate to rename it (every wire and instance
  follows); click it and press Delete to remove it. Naming a pin
  `data:8` (on creation or rename) makes it a **bundle pin** carrying 8
  lines; renaming with a new `:n` re-sizes it (its wires disconnect —
  the kind changed).
- **Closed-rectangle layout**: the “Layout” button opens a dialog where
  you drag each port to any edge (top and bottom included); width and
  height have sliders below (click a value to type it) — make a tiny XOR
  or a big processor-core slab. A resize that would make any placed
  instance overlap a neighbour is refused. Ports always sit on grid slots, so wires
  line up across components; the rectangle grows itself when a side needs
  another slot. Until you hand-place a port, inputs auto-stack on the
  left and outputs on the right; your first manual placement freezes the
  rest exactly where they are.
- **Rename / delete the abstraction**: the toolbar buttons inside its
  sheet (or click the title). Deleting is refused while instances of it
  are still placed somewhere.
- Instances everywhere update immediately; nesting is unlimited and
  recursion is refused. Clocks are welcome inside abstractions — every clk
  anywhere is the same shared signal.

## Running

“▶ Run” (or Space) compiles and starts the simulation without moving your
view — if you're zoomed three levels deep, you watch the signals right
there. Problems show in the status bar and you stay in edit mode. While running: ←/→ step the
clock back and forward (Home rewinds), or press **P** (or “Play”) to let
it run on its own — stepping by hand pauses it. `P` from edit mode starts
running and playing in one go. Wires glow amber when high, flip-flops fill
amber while storing a 1, every primitive shows its value badge, and
editing is locked until you stop (Esc, Space, or “■ Stop”). You can zoom
into abstractions while the simulation runs — their internals light up
with live values too.

Hotkeys are printed right on the toolbar buttons.

The clock starts at 0 and toggles once per step; flip-flops latch on the
rising edge. State comes from DFF `init` values — set them in edit mode by
double-clicking.

Clock components are all one shared signal: place as many as you like,
anywhere (including inside abstractions, handy for testing a part on its
own) — they tick in unison.
