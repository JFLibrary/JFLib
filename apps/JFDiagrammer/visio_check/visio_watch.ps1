# visio_watch.ps1 -- started hidden by visio_worker.vbs in the same desktop session.  Every dialog the
# job's Visio shows (the pid in JOB\visio.pid) is written to JOB\alerts.txt with its title and texts
# and then answered with Enter (its default button; WM_CLOSE when it is still there 4 s later).  It
# never talks to Visio through COM; it ends when JOB\done exists.
param([Parameter(Mandatory)] [string]$Job)
Add-Type @'
using System; using System.Text; using System.Runtime.InteropServices; using System.Collections.Generic;
public static class VW {
  public delegate bool P(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] static extern bool EnumWindows(P p, IntPtr l);
  [DllImport("user32.dll")] static extern bool EnumChildWindows(IntPtr h, P p, IntPtr l);
  [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetClassName(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
  [DllImport("user32.dll")] public static extern bool IsWindow(IntPtr h);
  public static string Cls(IntPtr h) { var c = new StringBuilder(256); GetClassName(h, c, 256); return c.ToString(); }
  public static string Txt(IntPtr h) { var t = new StringBuilder(2048); GetWindowText(h, t, 2048); return t.ToString(); }
  // visible top-level windows of the process that are not its main window (VISIOA)
  public static List<IntPtr> Dialogs(uint want) {
    var r = new List<IntPtr>();
    EnumWindows((h, l) => { uint pid; GetWindowThreadProcessId(h, out pid);
      if (pid == want && IsWindowVisible(h) && Cls(h) != "VISIOA" && Cls(h) != "IME" && !Cls(h).StartsWith("MSCTF")) r.Add(h);
      return true; }, IntPtr.Zero);
    return r;
  }
  public static string Texts(IntPtr h) {
    var r = new List<string>();
    EnumChildWindows(h, (k, l) => { var t = Txt(k); if (t.Length > 0) r.Add(t.Replace("\r", " ").Replace("\n", " ")); return true; }, IntPtr.Zero);
    return string.Join(" | ", r);
  }
}
'@
$alerts = Join-Path $Job 'alerts.txt'
$pidFile = Join-Path $Job 'visio.pid'
$done = Join-Path $Job 'done'
$seen = @{}
Set-Content -Path (Join-Path $Job 'watch.ready') -Value 'ready' -Encoding ascii
$t0 = Get-Date
while (-not (Test-Path $done) -and ((Get-Date) - $t0).TotalMinutes -lt 60) {
    if (Test-Path $pidFile) {
        $vpid = [uint32](Get-Content $pidFile -Raw).Trim()
        foreach ($h in [VW]::Dialogs($vpid)) {
            $k = [string]$h
            if (-not $seen.ContainsKey($k)) {
                $seen[$k] = Get-Date
                $line = "{0:HH:mm:ss} dialog [{1}] '{2}': {3}" -f (Get-Date), [VW]::Cls($h), [VW]::Txt($h), [VW]::Texts($h)
                Add-Content -Path $alerts -Value $line -Encoding UTF8
                [void][VW]::PostMessage($h, 0x100, [IntPtr]0x0D, [IntPtr]0)   # WM_KEYDOWN Enter
                [void][VW]::PostMessage($h, 0x101, [IntPtr]0x0D, [IntPtr]0)   # WM_KEYUP
            } elseif (((Get-Date) - $seen[$k]).TotalSeconds -gt 4 -and [VW]::IsWindow($h)) {
                Add-Content -Path $alerts -Value ("{0:HH:mm:ss} dialog still open, closing it" -f (Get-Date)) -Encoding UTF8
                [void][VW]::PostMessage($h, 0x10, [IntPtr]0, [IntPtr]0)       # WM_CLOSE
                $seen[$k] = (Get-Date).AddSeconds(3600)
            }
        }
    }
    Start-Sleep -Milliseconds 300
}
