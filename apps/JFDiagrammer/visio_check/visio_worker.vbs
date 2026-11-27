' visio_worker.vbs -- runs on the Windows machine with Visio, in the logged-on user's desktop session
' (started by visio_run.ps1 through a scheduled task; Visio's COM server does not work from an SSH
' session, and PowerShell's COM client hangs in Documents.Open, so this is VBScript).
'
'   cscript //NoLogo //B visio_worker.vbs JOBDIR
'
' JOBDIR\in      the files to check (.vsdx .vssx .vstx .vsdm .vssm .vstm .vsd .vss .vst .vdx .vsx .vtx)
' JOBDIR\job.txt options, one key=value per line:
'                  save=same,old   copies saved by Visio: same = the file's own family (.vsdx/.vssx/.vstx),
'                                  old = Visio 2003-2010 binary (.vsd/.vss/.vst)
'                  pdf=1           the document as PDF
'                  pages=png,svg   every page exported (default png,svg; "none" for nothing)
'                  shapes=1        a shape list per file (default 1)
'                  script=NAME     a VBScript of the check in JOBDIR\in; it may define Sub Check(doc, base)
'                                  (called for each file after the generic part) and Sub AfterAll()
'                                  (called once at the end, before Visio quits)
' JOBDIR\out     report.txt (UTF-16), per file BASE-pN.png / .svg, BASE-visio.EXT, BASE.pdf, BASE-shapes.tsv
' JOBDIR\done    written last ("ok" or "fatal ...")
'
' Every file is opened read-only, without macros, not added to the recent files.  While it opens,
' Visio may ask (a repair, a missing file ...): visio_watch.ps1 writes each dialog down and answers it
' with its default button, the report shows it as ALERT under the file.  Afterwards every question is
' answered "No" without a dialog.  "changed on open" means Visio marked the document as modified right
' after opening it (it recomputed or fixed something).
Option Explicit

Dim fso, jobDir, inDir, outDir, app, rep, opts, extraLoaded
Set fso = CreateObject("Scripting.FileSystemObject")
jobDir = WScript.Arguments(0)
inDir = jobDir & "\in"
outDir = jobDir & "\out"
If Not fso.FolderExists(outDir) Then fso.CreateFolder outDir
Set rep = fso.CreateTextFile(outDir & "\report.txt", True, True)
Set opts = CreateObject("Scripting.Dictionary")

Sub L(m)
    rep.WriteLine FormatDateTime(Now, 3) & "  " & m
End Sub

Sub Finish(status)
    Dim d
    rep.Close
    Set d = fso.CreateTextFile(jobDir & "\done", True)
    d.WriteLine status
    d.Close
End Sub

Function Num(x)
    ' a number with a point, six decimals at most, whatever the locale
    Num = Replace(CStr(Round(x, 6)), ",", ".")
End Function

Function Clean(s)
    Clean = Replace(Replace(Replace(Replace(s, vbTab, " "), vbCr, " "), vbLf, " "), ChrW(65532), "")
    If Len(Clean) > 80 Then Clean = Left(Clean, 80) & "..."
End Function

Function Opt(k, dflt)
    If opts.Exists(k) Then Opt = opts(k) Else Opt = dflt
End Function

Function HasWord(list, w)
    HasWord = InStr("," & list & ",", "," & w & ",") > 0
End Function

Sub ReadJob()
    Dim f, line, p
    If Not fso.FileExists(jobDir & "\job.txt") Then Exit Sub
    Set f = fso.OpenTextFile(jobDir & "\job.txt", 1)
    Do Until f.AtEndOfStream
        line = Trim(Replace(f.ReadLine, vbCr, ""))
        p = InStr(line, "=")
        If p > 1 Then opts(LCase(Trim(Left(line, p - 1)))) = Trim(Mid(line, p + 1))
    Loop
    f.Close
End Sub

' ---------------------------------------------------------------- helpers a check script may use

' a document of the job by file name, opened read-write as a copy in out\ (so a check may change it)
Function OpenCopy(name)
    Dim src, dst
    src = inDir & "\" & name
    dst = outDir & "\" & fso.GetBaseName(name) & "-work." & fso.GetExtensionName(name)
    fso.CopyFile src, dst, True
    Set OpenCopy = app.Documents.OpenEx(dst, 8 + 128)
End Function

' every page of doc exported as BASE-pN.EXT for each EXT of exts ("png,svg")
Sub ExportPages(doc, base, exts)
    Dim i, e, pg
    On Error Resume Next
    For i = 1 To doc.Pages.Count
        Set pg = doc.Pages.Item(i)
        For Each e In Split(exts, ",")
            If e <> "" And e <> "none" Then
                Err.Clear
                pg.Export outDir & "\" & base & "-p" & Right("0" & i, 2) & "." & e
                If Err.Number <> 0 Then L "  export page " & i & " ." & e & ": ERROR " & Err.Description
            End If
        Next
    Next
End Sub

' doc saved by Visio as out\NAME (the extension decides the format)
Sub SaveCopy(doc, name)
    On Error Resume Next
    Err.Clear
    doc.SaveAs outDir & "\" & name
    If Err.Number <> 0 Then L "  save " & name & ": ERROR " & Err.Description Else L "  saved " & name
End Sub

' a cell's formula and value (internal units), "-" when the shape has no such cell
Function CellText(shp, cellName)
    On Error Resume Next
    Dim c
    Err.Clear
    If shp.CellExistsU(cellName, 0) = 0 Then CellText = "-": Exit Function
    Set c = shp.CellsU(cellName)
    CellText = c.FormulaU & " = " & Num(c.ResultIU)
    If Err.Number <> 0 Then CellText = "ERROR " & Err.Description
End Function

' ---------------------------------------------------------------- the generic part

Sub ShapeRows(f, pageIx, shapes, parentId, depth)
    Dim i, s, m, t
    On Error Resume Next
    For i = 1 To shapes.Count
        Set s = shapes.Item(i)
        m = ""
        Err.Clear
        If Not s.Master Is Nothing Then m = s.Master.NameU
        t = ""
        t = s.Text
        f.WriteLine pageIx & vbTab & s.ID & vbTab & parentId & vbTab & Clean(s.NameU) & vbTab & Clean(m) & vbTab & _
            s.Type & vbTab & s.OneD & vbTab & Num(s.CellsU("PinX").ResultIU) & vbTab & Num(s.CellsU("PinY").ResultIU) & vbTab & _
            Num(s.CellsU("Width").ResultIU) & vbTab & Num(s.CellsU("Height").ResultIU) & vbTab & Num(s.CellsU("Angle").ResultIU) & vbTab & _
            s.GeometryCount & vbTab & Clean(t)
        If depth < 4 And s.Type = 2 Then ShapeRows f, pageIx, s.Shapes, s.ID, depth + 1
    Next
End Sub

Sub Describe(doc, base)
    Dim i, pg, n, f, ds, r
    On Error Resume Next
    L "  type " & doc.Type & " (1 drawing, 2 stencil, 3 template), pages " & doc.Pages.Count & ", masters " & doc.Masters.Count & _
      ", styles " & doc.Styles.Count
    For i = 1 To doc.Pages.Count
        Set pg = doc.Pages.Item(i)
        L "  page " & i & " """ & pg.NameU & """" & IIf(pg.Background, " (background)", "") & ": shapes " & pg.Shapes.Count & _
          ", size " & Num(pg.PageSheet.CellsU("PageWidth").ResultIU) & " x " & Num(pg.PageSheet.CellsU("PageHeight").ResultIU) & " in"
    Next
    If doc.Type = 2 Then
        For i = 1 To doc.Masters.Count
            L "  master " & i & ": " & doc.Masters.Item(i).NameU & " | " & doc.Masters.Item(i).Name
        Next
    End If
    ' a recordset's rows and columns can make Visio reach for its data source (a file at another
    ' machine's path: Visio then waits for good), so only on request: data=1
    Err.Clear
    n = doc.DataRecordsets.Count
    If Err.Number = 0 And n > 0 Then
        L "  data recordsets: " & n
        If Opt("data", "0") = "1" Then
            For i = 1 To n
                Set ds = doc.DataRecordsets.Item(i)
                L "  data recordset " & i & " """ & ds.Name & """: command " & Clean(ds.CommandString)
                r = ds.GetDataRowIDs("")
                L "    rows " & (UBound(r) + 1) & ", columns " & ds.DataColumns.Count
            Next
        End If
    End If
    If Opt("shapes", "1") = "1" And doc.Type <> 2 Then
        Set f = fso.CreateTextFile(outDir & "\" & base & "-shapes.tsv", True, True)
        f.WriteLine "page" & vbTab & "id" & vbTab & "parent" & vbTab & "NameU" & vbTab & "master" & vbTab & "type" & vbTab & "1D" & vbTab & _
            "PinX" & vbTab & "PinY" & vbTab & "Width" & vbTab & "Height" & vbTab & "Angle" & vbTab & "geometry" & vbTab & "text"
        For i = 1 To doc.Pages.Count
            ShapeRows f, i, doc.Pages.Item(i).Shapes, 0, 0
        Next
        f.Close
    End If
End Sub

Function IIf(c, a, b)
    If c Then IIf = a Else IIf = b
End Function

Sub CheckFile(file)
    Dim base, ext, doc, t0, saved, fam, oldExt, sameExt
    base = fso.GetBaseName(file.Name)
    ext = LCase(fso.GetExtensionName(file.Name))
    L "FILE " & file.Name & " (" & file.Size & " bytes)"
    On Error Resume Next
    t0 = Timer
    Dim before: before = AlertCount()
    app.AlertResponse = 0          ' Visio asks; visio_watch.ps1 writes the question down and answers it
    Err.Clear
    Set doc = app.Documents.OpenEx(file.Path, 2 + 8 + 128)
    Dim openErr: openErr = Err.Number: Dim openMsg: openMsg = Err.Description
    app.AlertResponse = 7
    WScript.Sleep 300
    ReportAlerts before
    If openErr <> 0 Then
        L "  OPEN FAILED: " & openMsg
        Exit Sub
    End If
    saved = doc.Saved
    L "  opened in " & Num(Timer - t0) & " s" & IIf(saved, "", "; changed on open")
    Describe doc, base
    If doc.Type <> 2 Then ExportPages doc, base, Opt("pages", "png,svg")
    fam = doc.Type
    If fam = 2 Then
        sameExt = "vssx": oldExt = "vss"
    ElseIf fam = 3 Then
        sameExt = "vstx": oldExt = "vst"
    Else
        sameExt = "vsdx": oldExt = "vsd"
    End If
    If HasWord(Opt("save", ""), "same") Then SaveCopy doc, base & "-visio." & sameExt
    If HasWord(Opt("save", ""), "old") Then SaveCopy doc, base & "-visio." & oldExt
    If Opt("pdf", "0") = "1" And doc.Type <> 2 Then
        Err.Clear
        doc.ExportAsFixedFormat 1, outDir & "\" & base & ".pdf", 1, 0
        If Err.Number <> 0 Then L "  pdf: ERROR " & Err.Description Else L "  pdf written"
    End If
    If extraLoaded Then
        Err.Clear
        Execute "Check doc, base"
        If Err.Number <> 0 And Err.Number <> 13 Then L "  check script: ERROR " & Err.Number & " " & Err.Description
    End If
    doc.Saved = True
    doc.Close
End Sub

' ---------------------------------------------------------------- the dialogs visio_watch.ps1 saw

Function AlertLines()
    Dim st, t
    AlertLines = Array()
    If Not fso.FileExists(jobDir & "\alerts.txt") Then Exit Function
    Set st = CreateObject("ADODB.Stream")
    st.Type = 2: st.Charset = "utf-8": st.Open
    st.LoadFromFile jobDir & "\alerts.txt"
    t = st.ReadText: st.Close
    t = Replace(t, vbCr, "")
    If Right(t, 1) = vbLf Then t = Left(t, Len(t) - 1)
    If t <> "" Then AlertLines = Split(t, vbLf)
End Function

Function AlertCount()
    AlertCount = UBound(AlertLines()) + 1
End Function

Sub ReportAlerts(before)
    Dim a, i
    a = AlertLines()
    For i = before To UBound(a)
        L "  ALERT " & a(i)
    Next
End Sub

Sub StartWatch()
    Dim sh, w, k
    Set sh = CreateObject("WScript.Shell")
    w = fso.GetParentFolderName(WScript.ScriptFullName) & "\visio_watch.ps1"
    sh.Run "powershell.exe -NoProfile -ExecutionPolicy Bypass -WindowStyle Hidden -File """ & w & """ -Job """ & jobDir & """", 0, False
    For k = 1 To 100
        If fso.FileExists(jobDir & "\watch.ready") Then Exit Sub
        WScript.Sleep 200
    Next
    L "warning: the dialog watcher did not start; Visio's questions are answered with No and not reported"
End Sub

' the process ids of Visio's automation instances (",1,2,"); Application.ProcessID is not the
' process id the system knows, so the job's own Visio is the one that appeared with CreateObject
Function AutomationPids()
    Dim p, s
    s = ","
    For Each p In GetObject("winmgmts:\\.\root\cimv2").ExecQuery("select ProcessId, CommandLine from Win32_Process where Name='VISIO.EXE'")
        If InStr(p.CommandLine & "", "/Automation") > 0 Then s = s & p.ProcessId & ","
    Next
    AutomationPids = s
End Function

' ---------------------------------------------------------------- main
Dim file, extra, n, pidsBefore, pidsAfter, vpid, q
ReadJob
On Error Resume Next
pidsBefore = AutomationPids()
Set app = CreateObject("Visio.InvisibleApp")
If Err.Number <> 0 Then L "FATAL: Visio not started: " & Err.Description: Finish "fatal": WScript.Quit 1
pidsAfter = AutomationPids()
vpid = ""
For Each q In Split(pidsAfter, ",")
    If q <> "" And InStr(pidsBefore, "," & q & ",") = 0 And vpid = "" Then vpid = q
Next
If vpid <> "" Then
    Dim pidFile: Set pidFile = fso.CreateTextFile(jobDir & "\visio.pid", True): pidFile.WriteLine vpid: pidFile.Close
End If
L "Visio " & app.Version & " build " & app.Build & ", pid " & vpid
StartWatch
app.AlertResponse = 7
extraLoaded = False
extra = Opt("script", "")
If extra <> "" Then
    Err.Clear
    ExecuteGlobal fso.OpenTextFile(inDir & "\" & extra, 1).ReadAll
    If Err.Number <> 0 Then L "check script " & extra & ": ERROR loading: " & Err.Description Else extraLoaded = True: L "check script " & extra & " loaded"
    Err.Clear
End If
n = 0
For Each file In fso.GetFolder(inDir).Files
    Select Case LCase(fso.GetExtensionName(file.Name))
        Case "vsdx", "vssx", "vstx", "vsdm", "vssm", "vstm", "vsd", "vss", "vst", "vdx", "vsx", "vtx"
            CheckFile file
            n = n + 1
    End Select
Next
If extraLoaded Then
    Err.Clear
    Execute "AfterAll"
    Err.Clear
End If
L "files checked: " & n
app.Quit
Finish "ok"
