# visio_check: JFDiagrammer's files in a real Visio

A check against Visio itself ("Visio-Gegenprobe", PLAN.md › Testen) without anyone at the Visio
machine: the files go over SSH to a Windows machine with Visio, an invisible Visio opens each one
there, and the results come back.

```
python apps/JFDiagrammer/visio_check/visio_check.py --save same --out OUT FILE...
```

The machine comes from `--host USER@HOST` or `JFD_VISIO_HOST`, the SSH key from `--key` or
`JFD_VISIO_KEY`. Nothing about the machine is in the repo.

## What comes back (OUT, default `visio-out` next to the first file)

| File | What it holds |
|---|---|
| `report.txt` | per file: opened or `OPEN FAILED`, `ALERT` (a question Visio asked while opening, with its text), `changed on open`, pages with size and shape count, a stencil's masters, the number of data recordsets, errors of exports and saves, the lines of a check script; at the end `MISMATCH` lines |
| `BASE-pNN.png`, `.svg` | every page as Visio draws it (`--pages png,svg` or `none`); Visio crops a page export to its shapes |
| `BASE-shapes.tsv` | every shape (subshapes four levels deep): page, ID, parent, NameU, master, type, 1-D, PinX, PinY, Width, Height, Angle (internal units: inches, radians), geometry sections, text |
| `BASE-visio.EXT` | the file saved again by Visio: `--save same` (.vsdx/.vssx/.vstx), `--save old` (Visio 2003-2010 binary .vsd/.vss/.vst) |
| `BASE.pdf` | `--pdf` |

`MISMATCH`: Visio driven this way repairs a broken file without a word (a page cut in half opens
with the shapes it could read, a page whose part is missing as an empty page). For the XML formats
the script therefore counts pages, top-level shapes per page and masters in the file itself and
reports every difference to what Visio read. The exit code is 1 then, or when a file did not open.

`--data` adds each data recordset's command, rows and columns. Off by default: a recordset whose
source is a file at another machine's path can make Visio wait for it for good.

## Checks of your own

`--script CHECK.vbs` runs VBScript in the same Visio. It may define

- `Sub Check(doc, base)`: called for each file after the generic part (`doc` is open read-only);
- `Sub AfterAll()`: called once at the end;

and use the worker's helpers: `L "text"` (a line in report.txt), `OpenCopy(name)` (a file of the job
opened read-write as a copy in out\\), `ExportPages doc, base, "png"`, `SaveCopy doc, "name.vsdx"`,
`CellText(shape, "PinX")` (`formula = value`), `Num(x)`, `app` (the Visio application), `inDir`,
`outDir`. Files that are not Visio files (an .xlsx a drawing links to, ...) go along into the same
folder. Example: change a shape's data and see whether its geometry follows:

```vbscript
Sub Check(doc, base)
    Dim d, s
    Set d = OpenCopy(base & ".vsdx")
    Set s = d.Pages.Item(1).Shapes.ItemU("Pyramid")
    s.CellsU("Prop.Levels").FormulaU = "5"
    L "  levels 5: width " & CellText(s, "Width") & ", subshapes " & s.Shapes.Count
    d.Saved = True: d.Close
End Sub
```

## How it works, and why so

- `visio_check.py` (here): copies the files and the three scripts into a job folder of its own on
  the machine, runs `visio_run.ps1` over SSH, copies `out` back, counts the files' contents for
  the MISMATCH lines.
- `visio_run.ps1` (there, in the SSH session): Visio's COM server does not work in an SSH session
  (Windows session 0: Visio starts and dies at the first document), so a scheduled task of the job's
  own starts the worker in the logged-on user's desktop session (interactive logon, allowed on
  battery; schtasks' default waits for mains power). It removes the task again and ends the job's
  Visio if it is still running (only the pid the worker wrote, and only an automation instance).
- `visio_worker.vbs` (there, in the desktop session): VBScript, because PowerShell's COM client
  hangs in `Documents.Open` (Visio itself stays responsive). Each file is opened read-only, without
  macros and not added to the recent files.
- `visio_watch.ps1` (there, started hidden by the worker): writes down every dialog the job's Visio
  shows while a file opens and answers it with its default button.

Needs on the machine: Visio (tested with 16.0.14334), a logged-on user (Visio runs in that session,
invisibly), an account that may create its own scheduled tasks, Windows Script Host (VBScript).
Several runs may go at once: each has its own folder, task and Visio.
