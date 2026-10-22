"""mkicons.py -- JFNetworker's UI icons (own artwork) -> src/workspace/icons/icons_data.rs

    python apps/JFNetworker/mkicons.py

The tools and panels of the editor (selection, hand, connect, note, fit, grid, snapping,
minimap, devices, terminal, packet) and the kinds of ports (RJ45, SFP cage, serial, console,
USB, radio, WAN), drawn through ui::icon_hook in every style at private
code points from 0x10F100 (src/workspace/icons/mod.rs defines the JN_* constants).  24-grid
outline art with 1.6 unit strokes like the netlab style's icons (tools/vicon.py).  The device
symbols are a different pipeline: icons/*.svg -> tools/vsym.py -> symbols_data.rs.
"""
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent.parent / "tools"))
from vicon import circle, dot, emit_rust, fill, line, poly, rrect  # noqa: E402

W = 1.6

HAND = ("M8 13.5V6.5A1.5 1.5 0 0 1 11 6.5V11"
        "M11 10.5V5A1.5 1.5 0 0 1 14 5V10.5"
        "M14 10.5V6A1.5 1.5 0 0 1 17 6V11"
        "M17 11V8.5A1.5 1.5 0 0 1 20 8.5V14C20 18 17.3 21 13.5 21H12.5C9.9 21 8.5 19.8 7 17.8"
        "L4.6 14.4A1.6 1.6 0 0 1 7 12.2L8 13.5")

ICONS = [
    # the arrow pointer
    (["JN_SELECT"], poly([(6, 3.5), (6, 18.5), (10, 14.8), (12.6, 20.6), (15.2, 19.4), (12.6, 13.7), (17.8, 13.7)], True), W),
    (["JN_HAND"], HAND, W),
    # two devices joined by a cable
    (["JN_CONNECT"], rrect(3, 14.5, 6.5, 6, 1.2) + rrect(14.5, 3.5, 6.5, 6, 1.2)
     + "M9.5 17.5H11.5A1 1 0 0 0 12.5 16.5V7.5A1 1 0 0 1 13.5 6.5H14.5", W),
    # a card with a folded corner
    (["JN_NOTE"], "M5 4H19V15L14 20H5Z" + "M14 20V15H19" + line(8, 8.5, 16, 8.5) + line(8, 12, 13, 12), W),
    # four corners around a box
    (["JN_FIT"], "M4 9V5A1 1 0 0 1 5 4H9" + "M15 4H19A1 1 0 0 1 20 5V9" + "M20 15V19A1 1 0 0 1 19 20H15"
     + "M9 20H5A1 1 0 0 1 4 19V15" + rrect(8.5, 8.5, 7, 7, 1), W),
    (["JN_GRID"], rrect(4, 4, 16, 16, 1.5) + line(9.33, 4, 9.33, 20) + line(14.67, 4, 14.67, 20)
     + line(4, 9.33, 20, 9.33) + line(4, 14.67, 20, 14.67), W),
    # a horseshoe magnet
    (["JN_SNAP"], "M5.5 4V12A6.5 6.5 0 0 0 18.5 12V4" + "M9.5 4V12A2.5 2.5 0 0 0 14.5 12V4"
     + line(5.5, 7.5, 9.5, 7.5) + line(14.5, 7.5, 18.5, 7.5), W),
    # the view and a small overview in its corner
    (["JN_MINIMAP"], rrect(3, 4.5, 18, 15, 1.5) + rrect(12, 11.5, 6.5, 5.5, 0.8), W),
    # a switch with ports
    (["JN_DEVICES"], rrect(2.5, 7.5, 19, 9, 1.6) + fill(rrect(5.2, 10.8, 2.6, 2.6, 0.4)) + fill(rrect(9.2, 10.8, 2.6, 2.6, 0.4))
     + fill(rrect(13.2, 10.8, 2.6, 2.6, 0.4)) + dot(18.3, 12.1, 0.95), W),
    (["JN_TERMINAL"], rrect(3, 4.5, 18, 15, 2) + poly([(7, 9.5), (10, 12), (7, 14.5)]) + line(12, 15, 16.5, 15), W),
    # an envelope
    (["JN_PACKET"], rrect(3, 6, 18, 12, 1.5) + poly([(3.8, 7), (12, 13), (20.2, 7)]), W),
    # the kinds of ports: an RJ45 jack with its latch notch and contacts
    (["JN_PORT_RJ45"], "M4.5 5.5H19.5V15.5H16V19H8V15.5H4.5Z" + line(8, 8.5, 8, 10.5) + line(10.67, 8.5, 10.67, 10.5)
     + line(13.33, 8.5, 13.33, 10.5) + line(16, 8.5, 16, 10.5), W),
    # an optics cage with a duplex connector
    (["JN_PORT_SFP"], rrect(3, 7, 18, 10, 1.4) + circle(9, 12, 2)
     + circle(15, 12, 2), W),
    # a D-sub plug
    (["JN_PORT_SERIAL"], "M3.5 7.5H20.5L18.3 16.5H5.7Z" + dot(8, 10.5, 0.9) + dot(12, 10.5, 0.9) + dot(16, 10.5, 0.9)
     + dot(10, 13.5, 0.9) + dot(14, 13.5, 0.9), W),
    # a console port: a jack with a prompt
    (["JN_PORT_CONSOLE"], rrect(3.5, 5, 17, 14, 1.6) + poly([(7.5, 9.5), (10, 12), (7.5, 14.5)]) + line(12, 14.5, 16, 14.5), W),
    # a USB-A plug
    (["JN_PORT_USB"], rrect(4, 7.5, 16, 9, 1.2) + fill(rrect(7, 10, 3, 2.5, 0.3)) + fill(rrect(14, 10, 3, 2.5, 0.3)), W),
    # radio waves
    (["JN_PORT_RADIO"], "M4.5 10A10.5 10.5 0 0 1 19.5 10" + "M7.5 13.3A6.2 6.2 0 0 1 16.5 13.3" + dot(12, 17, 1.4), W),
    # a globe: the provider's line
    (["JN_PORT_WAN"], circle(12, 12, 8.5) + "M12 3.5C8 8 8 16 12 20.5C16 16 16 8 12 3.5"
     + line(3.5, 12, 20.5, 12), W),
    # the lab's metrics (the Metriken panel): a line on its axes
    (["JN_METRICS"], "M4 4V20H20" + poly([(7.5, 15.5), (11, 10.5), (14, 13), (19, 6.5)]), W),
]


if __name__ == "__main__":
    emit_rust(ICONS, HERE / "src" / "workspace" / "icons" / "icons_data.rs",
              header="icons_data -- JFNetworker's UI icons (own artwork; see apps/JFNetworker/mkicons.py)",
              const_path="super")
