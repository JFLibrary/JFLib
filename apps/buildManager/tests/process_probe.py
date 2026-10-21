"""A descendant that keeps writing, used to verify native process-tree cancellation."""
import pathlib
import subprocess
import sys
import time

if sys.argv[1] == 'child':
    path = pathlib.Path(sys.argv[2])
    while True:
        path.write_text(str(time.time()), encoding='utf-8')
        time.sleep(.02)
else:
    sys.stdin.readline()
    subprocess.Popen([sys.executable, __file__, 'child', sys.argv[2]])
    print('ready', flush=True)
    time.sleep(60)
