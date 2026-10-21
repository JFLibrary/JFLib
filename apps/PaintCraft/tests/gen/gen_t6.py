# generator of apps/PaintCraft/tests/tools_clone.uis / tools_recolor.uis (agent T6)
# {P x y} -> x | y << 16 (probe / hook argument), {W x y} -> window point "240+x,148+y"
import re, sys, importlib.util
spec = importlib.util.spec_from_file_location("ref", "apps/PaintCraft/tests/gen/ref_recolor.py")
def sub(t):
    t = re.sub(r"\{P (-?\d+) (-?\d+)\}", lambda m: str(int(m[1]) | int(m[2]) << 16), t)
    t = re.sub(r"\{W (-?\d+) (-?\d+)\}", lambda m: "%d,%d" % (240 + int(m[1]), 148 + int(m[2])), t)
    return t
for name in sys.argv[1:]:
    src = open("apps/PaintCraft/tests/gen/" + name + ".tpl", encoding="utf-8").read()
    open("apps/PaintCraft/tests/" + name + ".uis", "w", encoding="utf-8", newline="\n").write(sub(src))
    print("wrote", name)
