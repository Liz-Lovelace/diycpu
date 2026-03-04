use femtovg::{Canvas, renderer::OpenGl};

use crate::microchip::draw_microchip;
use crate::wire::draw_wire;
use crate::workbench::Workbench;

/// Draw a visual test scene with various component sizes and wire states.
pub fn draw_demo(canvas: &mut Canvas<OpenGl>, wb: &Workbench) {
    // -- Microchip variations --

    // Tiny 2x2
    draw_microchip(canvas, wb, 1.0, 1.0, 2.0, 2.0);

    // Standard 3x4
    draw_microchip(canvas, wb, 5.0, 1.0, 3.0, 4.0);

    // Wide 5x3
    draw_microchip(canvas, wb, 10.0, 1.0, 5.0, 3.0);

    // Tall 2x6
    draw_microchip(canvas, wb, 17.0, 1.0, 2.0, 6.0);

    // Large 4x5
    draw_microchip(canvas, wb, 21.0, 1.0, 4.0, 5.0);

    // -- Wire variations --

    // Horizontal, not energized
    draw_wire(canvas, wb, (1.0, 9.0), (6.0, 9.0), false);

    // Horizontal, energized
    draw_wire(canvas, wb, (1.0, 10.5), (6.0, 10.5), true);

    // Vertical, not energized
    draw_wire(canvas, wb, (8.0, 8.0), (8.0, 13.0), false);

    // Vertical, energized
    draw_wire(canvas, wb, (9.5, 8.0), (9.5, 13.0), true);

    // Diagonal, not energized
    draw_wire(canvas, wb, (11.0, 8.0), (15.0, 12.0), false);

    // Diagonal, energized
    draw_wire(canvas, wb, (12.0, 8.0), (16.0, 12.0), true);

    // Short wire (single arrow)
    draw_wire(canvas, wb, (18.0, 9.0), (19.0, 9.0), false);

    // Short wire, energized
    draw_wire(canvas, wb, (18.0, 10.5), (19.0, 10.5), true);

    // -- Wired-up microchip scene --

    // A chip with wires going in/out
    draw_microchip(canvas, wb, 5.0, 15.0, 3.0, 4.0);

    // Input wires (energized and not)
    draw_wire(canvas, wb, (1.0, 15.5), (5.0, 15.5), true);
    draw_wire(canvas, wb, (1.0, 16.5), (5.0, 16.5), false);
    draw_wire(canvas, wb, (1.0, 17.5), (5.0, 17.5), true);
    draw_wire(canvas, wb, (1.0, 18.5), (5.0, 18.5), false);

    // Output wires
    draw_wire(canvas, wb, (8.0, 15.5), (13.0, 15.5), true);
    draw_wire(canvas, wb, (8.0, 16.5), (13.0, 16.5), true);
    draw_wire(canvas, wb, (8.0, 17.5), (13.0, 17.5), false);
    draw_wire(canvas, wb, (8.0, 18.5), (13.0, 18.5), false);

    // Two chips connected
    draw_microchip(canvas, wb, 16.0, 15.0, 3.0, 3.0);
    draw_wire(canvas, wb, (19.0, 15.5), (22.0, 15.5), true);
    draw_wire(canvas, wb, (19.0, 16.5), (22.0, 16.5), false);
    draw_wire(canvas, wb, (19.0, 17.5), (22.0, 17.5), true);
    draw_microchip(canvas, wb, 22.0, 15.0, 3.0, 3.0);
}
