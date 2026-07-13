# GUI manual

```
cargo run --features gui --bin diycpu-gui -- [project.json]
```

Extra flags for headless work: `--screenshot out.png` (render one frame and
exit), `--run N` (enter run mode and step N times first), `--sheet name`
(open an abstraction's sheet), `--size WxH`.

## Editing

| Action | How |
|---|---|
| Place a component | Click it in the palette, then click the canvas. Shift-click the canvas to place several. |
| Move | Drag a component. Multi-selections move together. |
| Select | Click a component; Ctrl-click to add/remove; drag on empty canvas for a box select; click a wire to select its net. |
| Wire | Click or drag from any port. Click a compatible port to connect; click empty canvas to drop a **node** and keep drawing from it; right-click or Esc to stop (everything placed stays). Wiring an already-connected input replaces its old connection. |
| Wire nodes | Click a node to draw a new branch from it (fan-out). Drag a node to move it. Hover a node and press Delete to remove it — the wire splits where it was cut. Stubs are fine: wires don't need to connect anything to exist. |
| Delete | Select, then Delete/Backspace. Deleting a component cleans up its connections but leaves your routed wires as stubs. |
| Flip a DFF's start value | Double-click it — the big digit on its face is the stored bit. |
| Notes | Pick “note” in the palette, click the canvas, type. Double-click a note to edit it (empty text removes it); drag to move; notes can sit anywhere. |
| Duplicate | Ctrl+D. |
| Undo / redo | Ctrl+Z / Ctrl+Shift+Z (or Ctrl+Y). One action = one undo step; a cancelled gesture never becomes one. |
| Cancel anything | Esc or right-click. |
| Pan / zoom | Middle- or right-drag; scroll wheel zooms at the cursor. `F` fits the circuit, Ctrl+0 resets zoom. |
| Files | Ctrl+S save, Ctrl+Shift+S save as, Ctrl+O open, Ctrl+N new — native file dialogs; unsaved changes ask you to repeat the action to confirm. |
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
  into an output pin to define what it emits. The vertical order of the
  pins inside is the port order on the closed rectangle outside.
  Double-click a pin's name plate to rename it (every wire and instance
  follows); click it and press Delete to remove it.
- **Rename / delete the abstraction**: the toolbar buttons inside its
  sheet (or click the title). Deleting is refused while instances of it
  are still placed somewhere.
- Instances everywhere update immediately; nesting is unlimited and
  recursion is refused. Clocks are welcome inside abstractions — every clk
  anywhere is the same shared signal.

## Running

“▶ Run” (or Space) compiles and starts the simulation; problems show in
the status bar and you stay in edit mode. While running: ←/→ step the
clock back and forward (Home rewinds), or press **P** (or “Play”) to let
it run on its own — stepping by hand pauses it. `P` from edit mode starts
running and playing in one go. Wires glow amber when high, flip-flops fill
amber while storing a 1, every primitive shows its value badge, and
editing is locked until you stop (Esc, Space, or “■ Stop”).

Hotkeys are printed right on the toolbar buttons.

The clock starts at 0 and toggles once per step; flip-flops latch on the
rising edge. State comes from DFF `init` values — set them in edit mode by
double-clicking.

Clock components are all one shared signal: place as many as you like,
anywhere (including inside abstractions, handy for testing a part on its
own) — they tick in unison.
