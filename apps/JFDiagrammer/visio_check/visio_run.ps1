# visio_run.ps1 -- runs on the Windows machine with Visio (called over SSH by visio_check.py).
#
#   powershell -NoProfile -ExecutionPolicy Bypass -File visio_run.ps1 -Job C:\...\jfd-visio-check\job-... -Timeout 300
#
# Starts visio_worker.vbs once in the logged-on user's desktop session through a scheduled task of its
# own (interactive logon, allowed on battery; schtasks' default would wait for mains power), waits for
# JOB\done, removes the task and ends the job's own Visio if it is still there (the pid the worker
# wrote, only when its command line says /Automation: never a Visio the user opened).  PowerShell
# itself never talks to Visio: its COM client hangs in Documents.Open.
param([Parameter(Mandatory)] [string]$Job, [int]$Timeout = 300)

$worker = Join-Path $PSScriptRoot 'visio_worker.vbs'
$name = 'jfd-visio-' + (Split-Path $Job -Leaf)
$done = Join-Path $Job 'done'
if (Test-Path $done) { Remove-Item $done }
$user = "$env:COMPUTERNAME\$env:USERNAME"
if (-not (Get-CimInstance Win32_LogonSession -Filter 'LogonType=2' -ErrorAction SilentlyContinue)) {
    'warning: no interactive logon found; the task starts only while the user is logged on'
}
$act = New-ScheduledTaskAction -Execute 'cscript.exe' -Argument "//NoLogo //B `"$worker`" `"$Job`""
$prin = New-ScheduledTaskPrincipal -UserId $user -LogonType Interactive
$set = New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries -ExecutionTimeLimit (New-TimeSpan -Seconds ($Timeout + 60))
Register-ScheduledTask -TaskName $name -Action $act -Principal $prin -Settings $set -Force | Out-Null
try {
    Start-ScheduledTask -TaskName $name
    $t0 = Get-Date
    while (-not (Test-Path $done) -and ((Get-Date) - $t0).TotalSeconds -lt $Timeout) { Start-Sleep -Seconds 2 }
    if (Test-Path $done) { "status: " + (Get-Content $done -Raw).Trim() + " after $([int]((Get-Date) - $t0).TotalSeconds) s" }
    else { "status: timeout after $Timeout s (task state $((Get-ScheduledTask -TaskName $name).State))" }
} finally {
    Unregister-ScheduledTask -TaskName $name -Confirm:$false -ErrorAction SilentlyContinue
    $pidFile = Join-Path $Job 'visio.pid'
    if (Test-Path $pidFile) {
        $vpid = [int](Get-Content $pidFile -Raw).Trim()
        $p = Get-CimInstance Win32_Process -Filter "ProcessId=$vpid" -ErrorAction SilentlyContinue
        if ($p -and $p.Name -eq 'VISIO.EXE' -and $p.CommandLine -match '/Automation') {
            "ending the job's Visio (pid $vpid)"
            Stop-Process -Id $vpid -Force -ErrorAction SilentlyContinue
        }
    }
}
