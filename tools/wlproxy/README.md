# wlproxy -- the plat-wayland acceptance tools (agent plat-wayland, R4)

wlinject.py is a Wayland protocol proxy inside WSL that plays pointer, keyboard and the other
clipboard clients for a real app window, so the user's input and system clipboard stay out.
accept.py reruns the whole comparison of linuxtest (asm build/rwref vs Rust build) on Wayland.
The scripts expect to live in build/plat-wayland/ (they write their outputs there):

    mkdir -p build/plat-wayland && cp tools/wlproxy/* build/plat-wayland/
    python build/plat-wayland/accept.py

keymap.xkb is the keymap WSLg's compositor sends, compiled from xkeyboard-config data; its
copyright and permission notices are in xkeyboard-config-COPYING.txt (THIRD_PARTY_NOTICES.md).

popflip.py (ROADMAP 6) runs on its own inside WSL from the repository root: a proxy that plays a
compositor which moves popups (it rewrites xdg_popup.configure), checked against the gallery's
combo box list (--popup-edges-open): the app lays the popup out where the compositor put it.

    python3 tools/wlproxy/popflip.py [--exe build/linux/gallery] [--shift 120]
