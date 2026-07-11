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
| Wire | Click or drag from any port to a compatible port. Click empty canvas mid-wire to add a bend. Wiring an already-connected input replaces its old connection. |
| Delete | Select, then Delete/Backspace. Deleting a component cleans up all its wires. |
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
clock back and forward (Home rewinds), wires glow amber when high, every
primitive shows its value badge, and editing is locked until you stop
(Esc, Space, or “■ Stop”).

The clock starts at 0 and toggles once per step; flip-flops latch on the
rising edge. State comes from DFF `init` values — set them in edit mode by
double-clicking.
