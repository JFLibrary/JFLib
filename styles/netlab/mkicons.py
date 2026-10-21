"""mkicons.py -- the NetLab style's vector icons (own artwork) -> icons_data.rs

    python styles/netlab/mkicons.py

Outline icons on a 24 grid with 1.6 unit strokes (round caps, the vicon renderer's only
kind); small corner radii and straight segments for a sober, technical look; filled variants
where the IC_* constant asks for one.  Code points not listed here fall back to the symbol
font (jfOffice's icons.bin).
"""
import math
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent.parent / "tools"))
from vicon import (arc, arc_arrow, circle, cross_poly, dot, emit_rust, evenodd, fill, line, poly,  # noqa: E402
                   rrect)

W = 1.6     # the standard stroke
WC = 1.2    # window caption glyphs (drawn small)
WM = 1.9    # check marks and chevrons


def gear(cx, cy, r_out, r_in, teeth, duty=0.5):
    """a closed gear outline: `teeth` flat-topped teeth between r_in and r_out"""
    pts = []
    step = 360.0 / teeth
    for k in range(teeth):
        a = k * step - 90
        half = step * duty / 2
        ramp = step * 0.12
        for deg, r in ((a - half - ramp, r_in), (a - half, r_out), (a + half, r_out), (a + half + ramp, r_in)):
            t = math.radians(deg)
            pts.append((cx + r * math.cos(t), cy + r * math.sin(t)))
    return poly(pts, True)


def folder():
    return "M3.5 6.5V18.5A1 1 0 0 0 4.5 19.5H19.5A1 1 0 0 0 20.5 18.5V8.5A1 1 0 0 0 19.5 7.5H11.5L9.5 5.5H4.5A1 1 0 0 0 3.5 6.5Z"


def page():
    return "M5.5 3.5H14L18.5 8V20.5H5.5Z" + "M13.5 3.5V8.5H18.5"


def eye():
    return "M2.5 12C4.8 8 8.2 6 12 6S19.2 8 21.5 12C19.2 16 15.8 18 12 18S4.8 16 2.5 12Z" + circle(12, 12, 2.8)


def clock(cx, cy, r):
    return circle(cx, cy, r) + poly([(cx, cy - r * 0.6), (cx, cy), (cx + r * 0.45, cy + r * 0.3)])


STAR = [(12, 3.6), (14.6, 9), (20.4, 9.6), (16, 13.5), (17.3, 19.4), (12, 16.4), (6.7, 19.4), (8, 13.5), (3.6, 9.6), (9.4, 9)]

ICONS = [
    # ---- basic signs
    (["IC_ADD"], line(12, 5, 12, 19) + line(5, 12, 19, 12), W),
    (["IC_REMOVE"], line(5, 12, 19, 12), W),
    (["IC_CANCEL", "IC_CHROME_CLOSE", "IC_CLEAR"], line(6.5, 6.5, 17.5, 17.5) + line(17.5, 6.5, 6.5, 17.5), W),
    (["IC_CHECK_MARK", "IC_ACCEPT"], poly([(5, 12.4), (9.6, 17), (19, 7.6)]), WM),
    (["IC_CHEV_DOWN", "IC_CHEV_DOWN_SMALL"], poly([(6.5, 9.5), (12, 15), (17.5, 9.5)]), WM),
    (["IC_CHEV_UP", "IC_CHEV_UP_SMALL"], poly([(6.5, 14.5), (12, 9), (17.5, 14.5)]), WM),
    (["IC_CHEV_LEFT", "IC_CHEV_LEFT_SMALL"], poly([(14.5, 6.5), (9, 12), (14.5, 17.5)]), WM),
    (["IC_CHEV_RIGHT", "IC_CHEV_RIGHT_SMALL"], poly([(9.5, 6.5), (15, 12), (9.5, 17.5)]), WM),
    (["IC_CHEV_UP_DOWN"], poly([(8, 9.5), (12, 5.5), (16, 9.5)]) + poly([(8, 14.5), (12, 18.5), (16, 14.5)]), W),
    (["IC_CARET_DOWN"], fill(poly([(7, 9.5), (17, 9.5), (12, 15)], True)), W),
    (["IC_CARET_UP"], fill(poly([(7, 14.5), (17, 14.5), (12, 9)], True)), W),
    (["IC_CARET_LEFT"], fill(poly([(14.5, 7), (14.5, 17), (9, 12)], True)), W),
    (["IC_CARET_RIGHT"], fill(poly([(9.5, 7), (9.5, 17), (15, 12)], True)), W),
    (["IC_BACK", "IC_PREVIOUS"], line(19.5, 12, 5, 12) + poly([(10.5, 6.5), (5, 12), (10.5, 17.5)]), W),
    (["IC_FORWARD", "IC_NEXT"], line(4.5, 12, 19, 12) + poly([(13.5, 6.5), (19, 12), (13.5, 17.5)]), W),
    (["IC_UP"], line(12, 19.5, 12, 5) + poly([(6.5, 10.5), (12, 5), (17.5, 10.5)]), W),
    (["IC_DOWN"], line(12, 4.5, 12, 19) + poly([(6.5, 13.5), (12, 19), (17.5, 13.5)]), W),
    (["IC_MENU"], line(4.5, 6.5, 19.5, 6.5) + line(4.5, 12, 19.5, 12) + line(4.5, 17.5, 19.5, 17.5), W),
    (["IC_MORE"], fill(rrect(4, 10.5, 3, 3, 0.6) + rrect(10.5, 10.5, 3, 3, 0.6) + rrect(17, 10.5, 3, 3, 0.6)), W),
    (["IC_UNDO"], poly([(8.5, 5.5), (4.5, 9.5), (8.5, 13.5)]) + "M4.5 9.5H14.5A5 5 0 0 1 14.5 19.5H10", W),
    (["IC_REDO"], poly([(15.5, 5.5), (19.5, 9.5), (15.5, 13.5)]) + "M19.5 9.5H9.5A5 5 0 0 0 9.5 19.5H14", W),
    (["IC_REFRESH"], arc_arrow(12, 12, 7.5, 320, 270, 3.6), W),
    (["IC_RETURN"], "M19 5.5V11.5A2 2 0 0 1 17 13.5H5.5" + poly([(9.5, 9.5), (5.5, 13.5), (9.5, 17.5)]), W),
    (["IC_BACKSPACE"], "M8.5 5.5H19.5A1 1 0 0 1 20.5 6.5V17.5A1 1 0 0 1 19.5 18.5H8.5L3 12Z"
     + line(11, 9.5, 16, 14.5) + line(16, 9.5, 11, 14.5), W),
    # ---- search and zoom
    (["IC_SEARCH", "IC_ZOOM"], circle(10.5, 10.5, 6) + line(15, 15, 20, 20), W),
    (["IC_ZOOM_IN"], circle(10.5, 10.5, 6) + line(15, 15, 20, 20) + line(10.5, 8, 10.5, 13) + line(8, 10.5, 13, 10.5), W),
    (["IC_ZOOM_OUT"], circle(10.5, 10.5, 6) + line(15, 15, 20, 20) + line(8, 10.5, 13, 10.5), W),
    (["IC_FILTER"], "M4 5H20L14 12.5V19L10 20.5V12.5Z", W),
    (["IC_SORT"], line(7.5, 5, 7.5, 19) + poly([(4.5, 16), (7.5, 19), (10.5, 16)])
     + line(16.5, 19, 16.5, 5) + poly([(13.5, 8), (16.5, 5), (19.5, 8)]), W),
    # ---- tools
    (["IC_SETTINGS"], gear(12, 12, 9.2, 7.2, 8) + circle(12, 12, 3), 1.5),
    (["IC_EDIT", "IC_PENCIL"], "M4.5 19.5L5.3 15.5L15.8 5A1.4 1.4 0 0 1 17.8 5L19 6.2A1.4 1.4 0 0 1 19 8.2L8.5 18.7Z"
     + line(14, 6.8, 17.2, 10), W),
    (["IC_DELETE"], line(4, 6.5, 20, 6.5) + "M9.5 6.5V4H14.5V6.5" + "M6.5 6.5L7.4 20.5H16.6L17.5 6.5"
     + line(10.2, 10.5, 10.2, 16.5) + line(13.8, 10.5, 13.8, 16.5), W),
    (["IC_SAVE", "IC_SAVE_AS", "IC_SAVE_COPY"], "M4 4H16.5L20 7.5V20H4Z" + "M8 4V8.5H15V4" + rrect(7.5, 13, 9, 7, 0), W),
    (["IC_COPY"], rrect(8.5, 8.5, 11.5, 11.5, 1) + "M15.5 8.5V5A1 1 0 0 0 14.5 4H5A1 1 0 0 0 4 5V14.5A1 1 0 0 0 5 15.5H8.5", W),
    (["IC_CUT"], circle(7, 17.5, 2.6) + circle(17, 17.5, 2.6) + line(8.6, 15.4, 16.5, 4) + line(15.4, 15.4, 7.5, 4), W),
    (["IC_PASTE"], "M9 5H5.5A1 1 0 0 0 4.5 6V20A1 1 0 0 0 5.5 21H18.5A1 1 0 0 0 19.5 20V6A1 1 0 0 0 18.5 5H15"
     + rrect(9, 3.5, 6, 3.5, 0.6), W),
    (["IC_SELECT_ALL"], "M4 7V4H7M10.5 4H13.5M17 4H20V7M20 10.5V13.5M20 17V20H17M13.5 20H10.5M7 20H4V17M4 13.5V10.5"
     + rrect(8, 8, 8, 8, 0.5), W),
    (["IC_REPAIR"], "M14.6 4.2A4.5 4.5 0 0 0 9.9 10.2L4.2 15.9A1.9 1.9 0 0 0 6.9 18.6L12.6 12.9A4.5 4.5 0 0 0 18.6 8.2L16 10.8L13.4 10.4L13 7.8Z", W),
    (["IC_DEV_TOOLS"], poly([(8.5, 7), (3.5, 12), (8.5, 17)]) + poly([(15.5, 7), (20.5, 12), (15.5, 17)]) + line(13.5, 5, 10.5, 19), W),
    (["IC_PRINT"], "M7 9V4H17V9" + "M7 17H4.5V10A1 1 0 0 1 5.5 9H18.5A1 1 0 0 1 19.5 10V17H17" + rrect(7, 13.5, 10, 7, 0), W),
    (["IC_CALCULATOR"], rrect(5, 3.5, 14, 17, 1) + rrect(8, 6.5, 8, 3.5, 0)
     + dot(9, 13.5, 0.9) + dot(12, 13.5, 0.9) + dot(15, 13.5, 0.9) + dot(9, 17, 0.9) + dot(12, 17, 0.9) + dot(15, 17, 0.9), W),
    (["IC_RULER"], "M3.5 15.5L15.5 3.5L20.5 8.5L8.5 20.5Z" + line(7.5, 11.5, 9.5, 13.5) + line(10.5, 8.5, 12.5, 10.5)
     + line(13.5, 5.5, 15.5, 7.5), W),
    (["IC_SLIDERS"], line(4, 7, 20, 7) + line(4, 12, 20, 12) + line(4, 17, 20, 17)
     + fill(rrect(7.5, 5, 3, 4, 0.5) + rrect(13.5, 10, 3, 4, 0.5) + rrect(6, 15, 3, 4, 0.5)), W),
    # ---- media
    (["IC_PLAY"], fill("M7.5 5V19A0.7 0.7 0 0 0 8.6 19.6L19.2 12.6A0.7 0.7 0 0 0 19.2 11.4L8.6 4.4A0.7 0.7 0 0 0 7.5 5Z"), W),
    (["IC_PAUSE"], fill(rrect(6.5, 5, 3.8, 14, 0.6) + rrect(13.7, 5, 3.8, 14, 0.6)), W),
    (["IC_STOP"], fill(rrect(6, 6, 12, 12, 0.8)), W),
    (["IC_RECORD", "IC_CIRCLE_FILL"], fill(circle(12, 12, 6)), W),
    (["IC_CIRCLE_RING", "IC_RADIO_OFF"], circle(12, 12, 7.5), W),
    (["IC_VOLUME"], "M4 9.5H7.5L12 5.5V18.5L7.5 14.5H4Z" + arc(12.5, 12, 4, 300, 60) + arc(12.5, 12, 7.5, 305, 55), W),
    # ---- status
    (["IC_INFO"], circle(12, 12, 8.5) + line(12, 11, 12, 16.5) + dot(12, 7.9), W),
    (["IC_WARNING"], "M10.4 4.4L2.8 17.8A1.8 1.8 0 0 0 4.4 20.5H19.6A1.8 1.8 0 0 0 21.2 17.8L13.6 4.4A1.8 1.8 0 0 0 10.4 4.4Z"
     + line(12, 9.5, 12, 14) + dot(12, 17), W),
    (["IC_ERROR", "IC_CANCEL_CIRCLE"], circle(12, 12, 8.5) + line(9, 9, 15, 15) + line(15, 9, 9, 15), W),
    (["IC_HELP"], circle(12, 12, 8.5) + "M9.5 9.6A2.5 2.5 0 0 1 14.5 9.9C14.5 11.6 12 12 12 13.8" + dot(12, 16.7), W),
    (["IC_CHECK_CIRCLE_FILL"], evenodd(circle(12, 12, 9) + poly([(7.3, 12.3), (8.7, 10.9), (10.7, 12.9), (15.3, 8.3), (16.7, 9.7), (10.7, 15.7)], True)), W),
    (["IC_ERROR_FILL"], evenodd(circle(12, 12, 9) + cross_poly(12, 12, 4.4, 1.0)), W),
    (["IC_ADD_CIRCLE"], circle(12, 12, 8.5) + line(12, 8, 12, 16) + line(8, 12, 16, 12), W),
    (["IC_REMOVE_CIRCLE"], circle(12, 12, 8.5) + line(8, 12, 16, 12), W),
    (["IC_BELL"], "M6 16.5V11A6 6 0 0 1 18 11V16.5L19.5 18H4.5Z" + "M10 20.5H14", W),
    (["IC_FLAG"], line(5.5, 21, 5.5, 3.5) + "M5.5 4.5H18L15.5 8.5L18 12.5H5.5", W),
    (["IC_LIGHTBULB"], "M9 17V15.3C7.2 14.2 6 12.4 6 10.2A6 6 0 0 1 18 10.2C18 12.4 16.8 14.2 15 15.3V17Z" + line(9.5, 20.5, 14.5, 20.5), W),
    (["IC_STAR"], poly(STAR, True), W),
    (["IC_STAR_FILL"], fill(poly(STAR, True)), W),
    # ---- places and files
    (["IC_HOME"], "M3.5 11.5L12 4L20.5 11.5" + "M6 9.5V20H10V14.5H14V20H18V9.5", W),
    (["IC_FOLDER", "IC_OPEN_FILE"], folder(), W),
    (["IC_FOLDER_OPEN"], "M3.5 18.5V6.5A1 1 0 0 1 4.5 5.5H9.5L11.5 7.5H17.5A1 1 0 0 1 18.5 8.5V10.5"
     + "M3.5 18.8L6.2 11.2A1 1 0 0 1 7.1 10.5H20.6A0.8 0.8 0 0 1 21.3 11.6L18.6 19A0.8 0.8 0 0 1 17.9 19.5H4.2A0.7 0.7 0 0 1 3.5 18.8Z", W),
    (["IC_NEW_FOLDER"], folder() + line(12, 10.5, 12, 16.5) + line(9, 13.5, 15, 13.5), W),
    (["IC_DOCUMENT", "IC_PAGE"], page() + line(8.5, 12.5, 15.5, 12.5) + line(8.5, 16, 13.5, 16), W),
    (["IC_DOWNLOAD"], line(12, 4, 12, 15) + poly([(7.5, 10.5), (12, 15), (16.5, 10.5)]) + "M4.5 15.5V20H19.5V15.5", W),
    (["IC_UPLOAD"], line(12, 15, 12, 4) + poly([(7.5, 8.5), (12, 4), (16.5, 8.5)]) + "M4.5 15.5V20H19.5V15.5", W),
    (["IC_HISTORY", "IC_RECENT"], arc_arrow(12, 12, 8.2, 300, 200, 3.4) + poly([(12, 7.5), (12, 12), (15, 14)]), W),
    (["IC_CALENDAR"], rrect(4, 5.5, 16, 15, 1) + line(4, 10, 20, 10) + line(8.5, 3.5, 8.5, 7) + line(15.5, 3.5, 15.5, 7)
     + dot(8.5, 14, 0.95) + dot(12, 14, 0.95) + dot(15.5, 14, 0.95) + dot(8.5, 17.2, 0.95) + dot(12, 17.2, 0.95), W),
    (["IC_PHOTO", "IC_IMAGE", "IC_PICTURES"], rrect(3.5, 5, 17, 14, 1) + circle(9, 10, 1.7)
     + poly([(4, 17.5), (9.5, 13), (13, 16), (16, 13.5), (20, 17)]), W),
    (["IC_LAYERS"], poly([(12, 4), (20.5, 8.5), (12, 13), (3.5, 8.5)], True) + poly([(3.5, 12.5), (12, 17), (20.5, 12.5)])
     + poly([(3.5, 16), (12, 20.5), (20.5, 16)]), W),
    (["IC_GRID", "IC_VIEW_ALL"], rrect(4, 4, 7, 7, 0.6) + rrect(13, 4, 7, 7, 0.6) + rrect(4, 13, 7, 7, 0.6) + rrect(13, 13, 7, 7, 0.6), W),
    (["IC_BULLETED_LIST"], line(9.5, 6.5, 20, 6.5) + line(9.5, 12, 20, 12) + line(9.5, 17.5, 20, 17.5)
     + fill(rrect(4, 5.3, 2.4, 2.4, 0.4) + rrect(4, 10.8, 2.4, 2.4, 0.4) + rrect(4, 16.3, 2.4, 2.4, 0.4)), W),
    # ---- people, links, network
    (["IC_PERSON"], circle(12, 8, 3.8) + "M4.5 20.5A7.5 7.5 0 0 1 19.5 20.5", W),
    (["IC_LOCK"], rrect(5, 10.5, 14, 10, 1) + "M8 10.5V7.5A4 4 0 0 1 16 7.5V10.5" + dot(12, 15.5, 1.3), W),
    (["IC_UNLOCK"], rrect(5, 10.5, 14, 10, 1) + "M8 10.5V7.5A4 4 0 0 1 15.7 6" + dot(12, 15.5, 1.3), W),
    (["IC_LINK"], "M10 14L14 10" + "M11 7L13 5A3.5 3.5 0 0 1 19 11L17 13" + "M13 17L11 19A3.5 3.5 0 0 1 5 13L7 11", W),
    (["IC_MAIL"], rrect(3.5, 5.5, 17, 13, 1) + poly([(4, 6.5), (12, 13), (20, 6.5)]), W),
    (["IC_GLOBE"], circle(12, 12, 8.5) + line(3.5, 12, 20.5, 12) + "M12 3.5C14.5 6 15.5 9 15.5 12S14.5 18 12 20.5C9.5 18 8.5 15 8.5 12S9.5 6 12 3.5Z", W),
    (["IC_SHARE"], circle(17.5, 6, 2.5) + circle(6.5, 12, 2.5) + circle(17.5, 18, 2.5) + line(8.7, 10.8, 15.3, 7.2) + line(8.7, 13.2, 15.3, 16.8), W),
    (["IC_POWER"], line(12, 3.5, 12, 11.5) + arc(12, 12.5, 7.5, 305, 235), W),
    (["IC_VIEW"], eye(), W),
    (["IC_HIDE"], eye() + line(4, 4, 20, 20), W),
    (["IC_PIN"], "M9 4H15L14 9.5L17 12.5V14H7V12.5L10 9.5Z" + line(12, 14, 12, 20.5), W),
    (["IC_SHUFFLE", "IC_SWITCH"], line(4, 8, 19, 8) + poly([(16, 5), (19, 8), (16, 11)]) + line(20, 16, 5, 16) + poly([(8, 13), (5, 16), (8, 19)]), W),
    (["IC_SCAN"], "M4 8V4H8M16 4H20V8M20 16V20H16M8 20H4V16" + line(7, 12, 17, 12), W),
    (["IC_MOVE"], line(12, 3.5, 12, 20.5) + line(3.5, 12, 20.5, 12) + poly([(9.5, 6), (12, 3.5), (14.5, 6)])
     + poly([(9.5, 18), (12, 20.5), (14.5, 18)]) + poly([(6, 9.5), (3.5, 12), (6, 14.5)]) + poly([(18, 9.5), (20.5, 12), (18, 14.5)]), W),
    (["IC_RESIZE"], line(5, 19, 19, 5) + poly([(13, 5), (19, 5), (19, 11)]) + poly([(5, 13), (5, 19), (11, 19)]), W),
    # ---- windows
    (["IC_OPEN_IN_NEW_WINDOW"], "M13.5 4H20V10.5" + line(20, 4, 11.5, 12.5) + "M18 14V20H4V6H10", W),
    (["IC_FULL_SCREEN"], "M4 9V4H9M15 4H20V9M20 15V20H15M9 20H4V15", W),
    (["IC_BACK_TO_WINDOW"], "M9 4V9H4M15 4V9H20M20 15H15V20M9 20V15H4", W),
    (["IC_CHROME_MIN"], line(5, 12, 19, 12), WC),
    (["IC_CHROME_MAX"], rrect(5, 5, 14, 14, 0), WC),
    (["IC_CHROME_RESTORE"], rrect(5, 8, 11, 11, 0) + "M8 8V5H19V16H16", WC),
]


if __name__ == "__main__":
    emit_rust(ICONS, HERE / "icons_data.rs",
              header="icons_data -- the NetLab style's vector icons (own artwork; see mkicons.py)",
              const_path="ui::keys")
