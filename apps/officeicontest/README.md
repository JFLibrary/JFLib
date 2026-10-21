This atlas exercises every Office semantic icon through the ordinary `Ui::draw_icon`
path, including the style hook, shared path/text renderer and disabled palette.
Each cell shows 16/20/32 DIP and a disabled 20-DIP symbol. IDs are listed in
`styles/jfoffice/ICON_ROLES.md`.

```text
python build_rs.py app officeicontest --target all --out build/wordcraft-icons
build/wordcraft-icons/win/officeicontest.exe --theme light --scale 150 --shot build/wordcraft-icons/atlas-light150.png
python apps/officeicontest/tests/native.py build/wordcraft-icons
node apps/officeicontest/tests/browser.mjs build/wordcraft-icons
```

The atlas is a visual acceptance tool, not a claim of pixel identity between
different text engines. Geometry and colour placements follow the licensed local
reference; rendering remains jfguiLib's native/portable implementation.

The native check compares all six light/dark × 100/150/200% renders byte for byte
between Windows and Linux, checks semantic colours and the disabled palette,
and runs live theme/accent/DPI changes under the incremental-render validator.
Its output hashes are saved under the selected build directory.

The browser check compares every icon against those native pixels in Chromium and
Firefox. All icons sit below the excluded 40-DIP platform-title strip. The native
test also verifies white Back/Forward icons on the accent background and palette
invalidation of a second window.
