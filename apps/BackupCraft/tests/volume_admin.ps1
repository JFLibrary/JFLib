# volume_admin.ps1 -- the volume backup through VSS on a small test volume (as administrator)
#
#   powershell -ExecutionPolicy Bypass -File apps\BackupCraft\tests\volume_admin.ps1
#
# Makes a 300 MB VHDX (diskpart), formats it NTFS under a free drive letter, writes a few files,
# runs volume_smoke.py --volume X: (VSS snapshot, restore, NTFS signature, second run with few
# new blocks), then detaches and deletes the VHDX -- also when the test fails.
# Needs build\win\bc-server.exe (python build_rs.py app BackupCraft) and Python.
#Requires -RunAsAdministrator
$ErrorActionPreference = 'Stop'
$root = (Resolve-Path "$PSScriptRoot\..\..\..").Path
$vhd = Join-Path $env:TEMP 'bc-volume-test.vhdx'
$used = (Get-PSDrive -PSProvider FileSystem).Name
$letter = [char[]](70..90) | Where-Object { $used -notcontains [string]$_ } | Select-Object -Last 1
if (-not $letter) { throw 'Kein freier Laufwerksbuchstabe' }
if (Test-Path $vhd) { Remove-Item $vhd -Force }

function DiskPart([string[]]$lines) {
    $f = Join-Path $env:TEMP 'bc-volume-test.diskpart'
    Set-Content -Path $f -Value $lines -Encoding ASCII
    $out = diskpart /s $f
    Remove-Item $f -Force
    if ($LASTEXITCODE -ne 0) { throw "diskpart: $out" }
}

Write-Host "Testvolume $($letter): aus $vhd"
DiskPart @("create vdisk file=`"$vhd`" maximum=300 type=expandable", "select vdisk file=`"$vhd`"", 'attach vdisk',
           'convert mbr', 'create partition primary', 'format fs=ntfs quick label=BCTEST', "assign letter=$letter")
$code = 1
try {
    1..20 | ForEach-Object { Set-Content -Path "$($letter):\datei$_.txt" -Value ("BackupCraft Volume-Test $_ " * 2000) }
    & python "$root\apps\BackupCraft\tests\volume_smoke.py" --server "$root\build\win\bc-server.exe" --volume "$($letter):"
    $code = $LASTEXITCODE
} finally {
    DiskPart @("select vdisk file=`"$vhd`"", 'detach vdisk')
    Remove-Item $vhd -Force -ErrorAction SilentlyContinue
}
exit $code
