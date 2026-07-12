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
| Flip a DFF's start value | Double-click it (badge shows 0/1). |
| Duplicate | Ctrl+D. |
| Undo / redo | Ctrl+Z / Ctrl+Shift+Z (or Ctrl+Y). One action = one undo step; a cancelled gesture never becomes one. |
| Cancel anything | Esc or right-click. |
| Pan / zoom | Middle- or right-drag; scroll wheel zooms at the cursor. `F` fits the circuit, Ctrl+0 resets zoom. |
| Save / new | Ctrl+S / Ctrl+N. The status bar confirms the path. |

## Abstractions

- **Create**: the “+ abstraction” button in the toolbar, then name it.
- **Edit**: double-click any instance (or create one) to open its sheet —
  the title bar shows `project ▸ name`. Esc or “◂ Back” returns.
- **Pins**: “+ input” / “+ output” add pins, which appear on rails beside
  the content. Wire from an input pin into the circuit like any port; wire
  a component output into an output pin to define what it emits. Click a
  pin's name plate and press Delete to remove it.
- Instances everywhere update immediately; nesting is unlimited, recursion
  is refused, and clocks can't be placed inside abstractions.

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
