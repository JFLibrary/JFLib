"""mkicons.py -- the BackupCraft style's vector icons (own artwork) -> icons_data.rs

    python styles/backupcraft/mkicons.py

Outline icons on a 24 grid with 1.7 unit strokes and round caps; filled variants where the
IC_* constant asks for one.  Code points not listed here fall back to the symbol font.
"""
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent.parent / "tools"))
from vicon import (arc, arc_arrow, circle, cross_poly, dot, emit_rust, evenodd, fill, line, poly,  # noqa: E402
                   rrect)

W = 1.7

GEAR = ("M18.94 10.08L21.24 10.29L21.24 13.71L18.94 13.92L18.27 15.55L19.75 17.32L17.32 19.75"
        "L15.55 18.27L13.92 18.94L13.71 21.24L10.29 21.24L10.08 18.94L8.45 18.27L6.68 19.75"
        "L4.25 17.32L5.73 15.55L5.06 13.92L2.76 13.71L2.76 10.29L5.06 10.08L5.73 8.45L4.25 6.68"
        "L6.68 4.25L8.45 5.73L10.08 5.06L10.29 2.76L13.71 2.76L13.92 5.06L15.55 5.73L17.32 4.25"
        "L19.75 6.68L18.27 8.45Z")


def eye():
    return ("M2.8 12C5 7.8 8.3 5.8 12 5.8S19 7.8 21.2 12C19 16.2 15.7 18.2 12 18.2S5 16.2 2.8 12Z"
            + circle(12, 12, 3))


def folder():
    return "M3.5 7V18A1.5 1.5 0 0 0 5 19.5H19A1.5 1.5 0 0 0 20.5 18V9.5A1.5 1.5 0 0 0 19 8H11.5L9.5 5.5H5A1.5 1.5 0 0 0 3.5 7Z"


def page():
    return "M6.5 3.5H14L18.5 8V19A1.5 1.5 0 0 1 17 20.5H6.5A1.5 1.5 0 0 1 5 19V5A1.5 1.5 0 0 1 6.5 3.5Z" + "M13.5 3.8V8.5H18.2"


ICONS = [
    (["IC_ADD"], line(12, 5, 12, 19) + line(5, 12, 19, 12), W),
    (["IC_REMOVE"], line(5, 12, 19, 12), W),
    (["IC_CANCEL", "IC_CHROME_CLOSE", "IC_CLEAR"], line(6.5, 6.5, 17.5, 17.5) + line(17.5, 6.5, 6.5, 17.5), W),
    (["IC_CHECK_MARK", "IC_ACCEPT"], poly([(5, 12.6), (9.7, 17.2), (19, 7.4)]), 2.0),
    (["IC_CHEV_DOWN", "IC_CHEV_DOWN_SMALL"], poly([(6, 9.5), (12, 15.5), (18, 9.5)]), 2.0),
    (["IC_CHEV_UP", "IC_CHEV_UP_SMALL"], poly([(6, 14.5), (12, 8.5), (18, 14.5)]), 2.0),
    (["IC_CHEV_LEFT", "IC_CHEV_LEFT_SMALL"], poly([(14.5, 6), (8.5, 12), (14.5, 18)]), 2.0),
    (["IC_CHEV_RIGHT", "IC_CHEV_RIGHT_SMALL"], poly([(9.5, 6), (15.5, 12), (9.5, 18)]), 2.0),
    (["IC_CHEV_UP_DOWN"], poly([(7.5, 9.5), (12, 5), (16.5, 9.5)]) + poly([(7.5, 14.5), (12, 19), (16.5, 14.5)]), W),
    (["IC_CARET_DOWN"], fill(poly([(6.5, 9.5), (17.5, 9.5), (12, 15.5)], True)), W),
    (["IC_CARET_UP"], fill(poly([(6.5, 14.5), (17.5, 14.5), (12, 8.5)], True)), W),
    (["IC_CARET_LEFT"], fill(poly([(14.5, 6.5), (14.5, 17.5), (8.5, 12)], True)), W),
    (["IC_CARET_RIGHT"], fill(poly([(9.5, 6.5), (9.5, 17.5), (15.5, 12)], True)), W),
    (["IC_BACK", "IC_PREVIOUS"], line(19.5, 12, 5, 12) + poly([(11, 6), (5, 12), (11, 18)]), W),
    (["IC_FORWARD", "IC_NEXT"], line(4.5, 12, 19, 12) + poly([(13, 6), (19, 12), (13, 18)]), W),
    (["IC_UP"], line(12, 19.5, 12, 5) + poly([(6, 11), (12, 5), (18, 11)]), W),
    (["IC_DOWN"], line(12, 4.5, 12, 19) + poly([(6, 13), (12, 19), (18, 13)]), W),
    (["IC_MENU"], line(4, 7, 20, 7) + line(4, 12, 20, 12) + line(4, 17, 20, 17), W),
    (["IC_MORE"], fill(circle(5.5, 12, 1.6) + circle(12, 12, 1.6) + circle(18.5, 12, 1.6)), W),
    (["IC_SEARCH", "IC_ZOOM"], circle(10.5, 10.5, 6.5) + line(15.3, 15.3, 20, 20), W),
    (["IC_ZOOM_IN"], circle(10.5, 10.5, 6.5) + line(15.3, 15.3, 20, 20) + line(10.5, 7.8, 10.5, 13.2) + line(7.8, 10.5, 13.2, 10.5), W),
    (["IC_ZOOM_OUT"], circle(10.5, 10.5, 6.5) + line(15.3, 15.3, 20, 20) + line(7.8, 10.5, 13.2, 10.5), W),
    (["IC_SETTINGS"], GEAR + circle(12, 12, 3.2), 1.6),
    (["IC_REFRESH"], arc_arrow(12, 12, 7.5, 330, 285, 3.8), W),
    (["IC_EDIT", "IC_PENCIL"], "M4.5 19.5L5.2 15.8L15.6 5.4A2 2 0 0 1 18.6 8.4L8.2 18.8Z" + line(14, 7, 17, 10), W),
    (["IC_DELETE"], line(4, 7, 20, 7) + "M9 7V5A1 1 0 0 1 10 4H14A1 1 0 0 1 15 5V7"
     + "M6.5 7L7.4 19A1.5 1.5 0 0 0 8.9 20.5H15.1A1.5 1.5 0 0 0 16.6 19L17.5 7" + line(10, 11, 10, 16.5) + line(14, 11, 14, 16.5), W),
    (["IC_SAVE", "IC_SAVE_AS", "IC_SAVE_COPY"], "M5 4H16L20 8V18.5A1.5 1.5 0 0 1 18.5 20H5.5A1.5 1.5 0 0 1 4 18.5V5A1 1 0 0 1 5 4Z"
     + "M8 4V8.5H15V4" + rrect(7.5, 13, 9, 7, 0.8), W),
    (["IC_COPY"], rrect(8.5, 8.5, 11.5, 11.5, 1.8) + "M15.5 8.5V5.8A1.8 1.8 0 0 0 13.7 4H5.8A1.8 1.8 0 0 0 4 5.8V13.7A1.8 1.8 0 0 0 5.8 15.5H8.5", W),
    (["IC_PASTE"], "M9 5H6.5A1.5 1.5 0 0 0 5 6.5V19A1.5 1.5 0 0 0 6.5 20.5H17.5A1.5 1.5 0 0 0 19 19V6.5A1.5 1.5 0 0 0 17.5 5H15"
     + rrect(9, 3.5, 6, 3.2, 1), W),
    (["IC_PLAY"], fill("M8 5.2V18.8A0.8 0.8 0 0 0 9.2 19.5L19.6 12.7A0.8 0.8 0 0 0 19.6 11.3L9.2 4.5A0.8 0.8 0 0 0 8 5.2Z"), W),
    (["IC_PAUSE"], fill(rrect(6.5, 5, 4, 14, 1) + rrect(13.5, 5, 4, 14, 1)), W),
    (["IC_STOP"], fill(rrect(6, 6, 12, 12, 1.5)), W),
    (["IC_RECORD", "IC_CIRCLE_FILL"], fill(circle(12, 12, 6)), W),
    (["IC_CIRCLE_RING", "IC_RADIO_OFF"], circle(12, 12, 7.5), W),
    (["IC_INFO"], circle(12, 12, 8.5) + line(12, 11, 12, 16.5) + dot(12, 7.9), W),
    (["IC_WARNING"], "M10.3 4.6L2.9 17.6A2 2 0 0 0 4.6 20.5H19.4A2 2 0 0 0 21.1 17.6L13.7 4.6A2 2 0 0 0 10.3 4.6Z"
     + line(12, 9.5, 12, 13.8) + dot(12, 16.9), W),
    (["IC_ERROR", "IC_CANCEL_CIRCLE"], circle(12, 12, 8.5) + line(9, 9, 15, 15) + line(15, 9, 9, 15), W),
    (["IC_HELP"], circle(12, 12, 8.5) + "M9.6 9.6A2.5 2.5 0 0 1 14.5 10C14.5 11.7 12 12 12 13.8" + dot(12, 16.6), W),
    (["IC_CHECK_CIRCLE_FILL"], evenodd(circle(12, 12, 9) + poly([(7.4, 12.4), (8.8, 11.0), (10.7, 12.9), (15.2, 8.4), (16.6, 9.8), (10.7, 15.7)], True)), W),
    (["IC_ERROR_FILL"], evenodd(circle(12, 12, 9) + cross_poly(12, 12, 4.4, 1.0)), W),
    (["IC_ADD_CIRCLE"], circle(12, 12, 8.5) + line(12, 8, 12, 16) + line(8, 12, 16, 12), W),
    (["IC_REMOVE_CIRCLE"], circle(12, 12, 8.5) + line(8, 12, 16, 12), W),
    (["IC_HOME"], "M4 11L12 4L20 11" + "M6 9.5V19A1 1 0 0 0 7 20H10V14.5H14V20H17A1 1 0 0 0 18 19V9.5", W),
    (["IC_HISTORY", "IC_RECENT"], arc_arrow(12, 12, 8.2, 300, 200, 3.4) + poly([(12, 7.5), (12, 12), (15, 14)]), W),
    (["IC_CALENDAR"], rrect(4, 5.5, 16, 14.5, 2) + line(4, 10, 20, 10) + line(8.5, 3.5, 8.5, 7) + line(15.5, 3.5, 15.5, 7)
     + dot(8.5, 14, 1.0) + dot(12, 14, 1.0) + dot(15.5, 14, 1.0), W),
    (["IC_FILTER"], "M4 5.5H20L14 12.8V18.5L10 20V12.8Z", W),
    (["IC_SORT"], line(7.5, 5, 7.5, 19) + poly([(4.5, 16), (7.5, 19), (10.5, 16)]) + line(16.5, 19, 16.5, 5) + poly([(13.5, 8), (16.5, 5), (19.5, 8)]), W),
    (["IC_FOLDER", "IC_OPEN_FILE"], folder(), W),
    (["IC_FOLDER_OPEN"], "M3.5 18V7A1.5 1.5 0 0 1 5 5.5H9.5L11.5 8H17.5A1.5 1.5 0 0 1 19 9.5V11"
     + "M3.6 18.4L6 12.3A1.5 1.5 0 0 1 7.4 11.3H20.4A1 1 0 0 1 21.3 12.7L18.9 18.6A1.5 1.5 0 0 1 17.5 19.5H4.6A1 1 0 0 1 3.6 18.4Z", W),
    (["IC_NEW_FOLDER"], folder() + line(12, 11, 12, 16.5) + line(9.2, 13.8, 14.8, 13.8), W),
    (["IC_DOCUMENT", "IC_PAGE"], page() + line(8.5, 12.5, 15, 12.5) + line(8.5, 16, 13, 16), W),
    (["IC_DOWNLOAD"], line(12, 4, 12, 15) + poly([(7.5, 10.5), (12, 15), (16.5, 10.5)]) + "M4.5 15.5V18.5A1.5 1.5 0 0 0 6 20H18A1.5 1.5 0 0 0 19.5 18.5V15.5", W),
    (["IC_UPLOAD"], line(12, 15, 12, 4) + poly([(7.5, 8.5), (12, 4), (16.5, 8.5)]) + "M4.5 15.5V18.5A1.5 1.5 0 0 0 6 20H18A1.5 1.5 0 0 0 19.5 18.5V15.5", W),
    (["IC_LOCK"], rrect(5, 10.5, 14, 10, 2) + "M8 10.5V7.5A4 4 0 0 1 16 7.5V10.5" + dot(12, 15.5, 1.3), W),
    (["IC_UNLOCK"], rrect(5, 10.5, 14, 10, 2) + "M8 10.5V7.5A4 4 0 0 1 15.6 5.8" + dot(12, 15.5, 1.3), W),
    (["IC_PERSON"], circle(12, 8, 3.8) + "M4.5 20.5A7.5 7.5 0 0 1 19.5 20.5", W),
    (["IC_BELL"], "M6 16.5V11A6 6 0 0 1 18 11V16.5L19.5 18H4.5Z" + "M10 20.5A2.2 2.2 0 0 0 14 20.5", W),
    (["IC_LINK"], "M10.5 13.5A3.5 3.5 0 0 0 15.4 13.6L18.3 10.7A3.5 3.5 0 0 0 13.3 5.7L12 7"
     + "M13.5 10.5A3.5 3.5 0 0 0 8.6 10.4L5.7 13.3A3.5 3.5 0 0 0 10.7 18.3L12 17", W),
    (["IC_MAIL"], rrect(3.5, 5.5, 17, 13, 2) + poly([(4, 7), (12, 13), (20, 7)]), W),
    (["IC_GLOBE"], circle(12, 12, 8.5) + line(3.5, 12, 20.5, 12) + "M12 3.5C14.6 6 15.6 9 15.6 12S14.6 18 12 20.5C9.4 18 8.4 15 8.4 12S9.4 6 12 3.5Z", W),
    (["IC_POWER"], line(12, 3.5, 12, 11.5) + arc(12, 12.5, 7.5, 305, 235), W),
    (["IC_VIEW"], eye(), W),
    (["IC_HIDE"], eye() + line(4, 4, 20, 20), W),
    (["IC_OPEN_IN_NEW_WINDOW"], "M13 4H20V11" + line(20, 4, 11.5, 12.5) + "M18 14V18.5A1.5 1.5 0 0 1 16.5 20H5.5A1.5 1.5 0 0 1 4 18.5V7.5A1.5 1.5 0 0 1 5.5 6H10", W),
    (["IC_CHROME_MIN"], line(5, 12, 19, 12), 1.3),
    (["IC_CHROME_MAX", "IC_FULL_SCREEN"], rrect(5, 5, 14, 14, 1.5), 1.3),
    (["IC_CHROME_RESTORE", "IC_BACK_TO_WINDOW"], rrect(5, 8, 11, 11, 1.5) + "M8 8V6.5A1.5 1.5 0 0 1 9.5 5H17.5A1.5 1.5 0 0 1 19 6.5V14.5A1.5 1.5 0 0 1 17.5 16H16", 1.3),
    (["IC_STAR"], poly([(12, 3.8), (14.5, 9), (20.2, 9.6), (16, 13.5), (17.1, 19.2), (12, 16.4), (6.9, 19.2), (8, 13.5), (3.8, 9.6), (9.5, 9)], True), W),
    (["IC_STAR_FILL"], fill(poly([(12, 3.8), (14.5, 9), (20.2, 9.6), (16, 13.5), (17.1, 19.2), (12, 16.4), (6.9, 19.2), (8, 13.5), (3.8, 9.6), (9.5, 9)], True)), W),
    (["IC_PIN"], "M9 4H15L14 9.5L17 12.5V14H7V12.5L10 9.5Z" + line(12, 14, 12, 20.5), W),
    (["IC_FLAG"], line(5.5, 21, 5.5, 3.5) + "M5.5 4.5H17L14.5 8.5L17 12.5H5.5", W),
    (["IC_REPAIR"], "M14.6 4.2A4.5 4.5 0 0 0 9.8 10.3L4 16.1A1.9 1.9 0 0 0 6.7 18.8L12.5 13A4.5 4.5 0 0 0 18.6 8.2L16 10.7L13.3 10.4L13 7.7Z", W),
    (["IC_SHARE"], circle(17.5, 6, 2.5) + circle(6.5, 12, 2.5) + circle(17.5, 18, 2.5) + line(8.7, 10.8, 15.3, 7.2) + line(8.7, 13.2, 15.3, 16.8), W),
    (["IC_GRID", "IC_VIEW_ALL"], rrect(4, 4, 7, 7, 1.2) + rrect(13, 4, 7, 7, 1.2) + rrect(4, 13, 7, 7, 1.2) + rrect(13, 13, 7, 7, 1.2), W),
    (["IC_BULLETED_LIST"], line(9.5, 6.5, 20, 6.5) + line(9.5, 12, 20, 12) + line(9.5, 17.5, 20, 17.5)
     + dot(5.2, 6.5, 1.2) + dot(5.2, 12, 1.2) + dot(5.2, 17.5, 1.2), W),
    (["IC_SLIDERS"], line(4, 7, 20, 7) + line(4, 12, 20, 12) + line(4, 17, 20, 17)
     + fill(circle(9, 7, 2.2) + circle(15, 12, 2.2) + circle(7.5, 17, 2.2)), W),
    (["IC_LIGHTBULB"], "M9 17.5V15.5C7.3 14.4 6 12.6 6 10.3A6 6 0 0 1 18 10.3C18 12.6 16.7 14.4 15 15.5V17.5Z" + line(9.5, 20.5, 14.5, 20.5), W),
    (["IC_DEV_TOOLS"], poly([(8.5, 7), (3.5, 12), (8.5, 17)]) + poly([(15.5, 7), (20.5, 12), (15.5, 17)]) + line(13.5, 5, 10.5, 19), W),
    (["IC_PRINT"], "M7 9V4H17V9" + "M7 17H5.5A1.5 1.5 0 0 1 4 15.5V10.5A1.5 1.5 0 0 1 5.5 9H18.5A1.5 1.5 0 0 1 20 10.5V15.5A1.5 1.5 0 0 1 18.5 17H17"
     + rrect(7, 14, 10, 6.5, 0.6), W),
    (["IC_SHUFFLE", "IC_SWITCH"], line(4, 8, 19, 8) + poly([(16, 5), (19, 8), (16, 11)]) + line(20, 16, 5, 16) + poly([(8, 13), (5, 16), (8, 19)]), W),
    (["IC_SCAN"], "M4 8V5.5A1.5 1.5 0 0 1 5.5 4H8M16 4H18.5A1.5 1.5 0 0 1 20 5.5V8M20 16V18.5A1.5 1.5 0 0 1 18.5 20H16M8 20H5.5A1.5 1.5 0 0 1 4 18.5V16"
     + line(7, 12, 17, 12), W),
    (["IC_LAYERS"], poly([(12, 4), (20.5, 8.5), (12, 13), (3.5, 8.5)], True) + poly([(3.5, 12.5), (12, 17), (20.5, 12.5)]) + poly([(3.5, 16), (12, 20.5), (20.5, 16)]), W),
    (["IC_PHOTO", "IC_IMAGE", "IC_PICTURES"], rrect(3.5, 5, 17, 14, 2) + circle(9, 10, 1.8) + poly([(4, 17.5), (9.5, 13), (13, 16), (16, 13.5), (20, 17)]), W),
]


if __name__ == "__main__":
    emit_rust(ICONS, HERE / "icons_data.rs",
              header="icons_data -- the BackupCraft style's vector icons (own artwork; see mkicons.py)",
              const_path="ui::keys")
