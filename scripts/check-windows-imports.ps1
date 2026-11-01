<#
.SYNOPSIS
  Fails if limusic-app.exe's load-time imports differ from .github\windows-exe-imports.txt.

.DESCRIPTION
  Every load-time import is a DLL the app needs just to START, on every machine, karaoke or not.
  The karaoke runtime (ONNX Runtime, DirectML, D3D12/DXGI, the C++ runtime) must never show up
  here: it is loaded by full path only when karaoke is turned on. A delay-load section fails too.

  Usage: scripts\check-windows-imports.ps1 -Exe target\release\limusic-app.exe
  Compare a build made the way releases are (the tauri CLI, or STATIC_VCRUNTIME=true): a plain
  `cargo build` links the VC runtime dynamically and adds vcruntime140*.dll.

  dumpbin is taken from PATH (a VS developer shell) or found with vswhere.
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$Exe,
    [string]$AllowList = ''
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version 2

if (-not $AllowList) { $AllowList = Join-Path (Split-Path -Parent $PSScriptRoot) '.github\windows-exe-imports.txt' }

function Find-Dumpbin {
    $cmd = Get-Command dumpbin.exe -ErrorAction SilentlyContinue
    if ($cmd) { return $cmd.Source }
    $vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
    if (-not (Test-Path -LiteralPath $vswhere)) { throw 'dumpbin.exe not on PATH and vswhere.exe not found' }
    $vs = & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
    if (-not $vs) { throw 'no Visual Studio with the C++ x64 tools found' }
    $found = Get-ChildItem -LiteralPath (Join-Path $vs 'VC\Tools\MSVC') -Directory |
        Sort-Object { [version]$_.Name } -Descending |
        ForEach-Object { Join-Path $_.FullName 'bin\Hostx64\x64\dumpbin.exe' } |
        Where-Object { Test-Path -LiteralPath $_ } |
        Select-Object -First 1
    if (-not $found) { throw 'dumpbin.exe not found under the Visual Studio C++ tools' }
    return $found
}

if (-not (Test-Path -LiteralPath $Exe)) { throw "$Exe not found" }
$dumpbin = Find-Dumpbin
Write-Verbose "dumpbin: $dumpbin"
$lines = & $dumpbin /NOLOGO /DEPENDENTS $Exe
if ($LASTEXITCODE -ne 0) { throw "dumpbin ($dumpbin) failed on $Exe with exit code $LASTEXITCODE" }

$imports = @()
$delay = @()
$section = $null
foreach ($line in $lines) {
    if ($line -match 'has the following delay load dependencies') { $section = 'delay'; continue }
    if ($line -match 'has the following dependencies') { $section = 'load'; continue }
    if ($line -match '^\s*Summary') { break }
    if ($section -and $line -match '^\s+(\S+\.dll)\s*$') {
        if ($section -eq 'load') { $imports += $Matches[1].ToLowerInvariant() } else { $delay += $Matches[1].ToLowerInvariant() }
    }
}
# dumpbin lists some DLLs twice in different case (kernel32.dll and KERNEL32.dll).
$imports = @($imports | Sort-Object -Unique)
if ($imports.Count -lt 10) { throw "parsed only $($imports.Count) imports from $Exe; dumpbin output format changed?" }

$allowed = Get-Content -LiteralPath $AllowList |
    ForEach-Object { ($_ -replace '#.*$', '').Trim().ToLowerInvariant() } |
    Where-Object { $_ }

$extra = @($imports | Where-Object { $allowed -notcontains $_ } | Sort-Object -Unique)
$missing = @($allowed | Where-Object { $imports -notcontains $_ } | Sort-Object -Unique)

$ok = $true
if ($extra.Count) { $ok = $false; Write-Host "NOT ALLOWED (new load-time imports): $($extra -join ', ')" }
if ($missing.Count) { $ok = $false; Write-Host "no longer imported (update the allow-list on purpose): $($missing -join ', ')" }
if ($delay.Count) { $ok = $false; Write-Host "delay-load imports are not allowed: $($delay -join ', ')" }
if (-not $ok) {
    Write-Host "allow-list: $AllowList"
    throw "$Exe imports differ from the allow-list"
}
Write-Host "$Exe imports $($imports.Count) DLLs, exactly the allow-list"
