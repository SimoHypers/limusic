<#
.SYNOPSIS
  Puts the Windows karaoke runtime into src-tauri\karaoke-runtime\ (gitignored), where
  tauri.windows.conf.json picks it up as a bundle resource.

.DESCRIPTION
  The exe links nothing from ONNX Runtime (ort `load-dynamic`): these files are shipped next to it,
  in a private subfolder, and loaded by full path only when karaoke is turned on.

  - onnxruntime.dll (official DirectML build 1.24.4) and DirectML.dll 1.15.4 come out of the
    official NuGet packages. Package and DLL are both checked against pinned SHA-256 values.
    The DirectML licence allows distributing it as part of an application, which is why it is
    bundled rather than downloaded by the app.
  - vcruntime140.dll, vcruntime140_1.dll, msvcp140.dll and msvcp140_1.dll (app-local VC++
    runtime, what onnxruntime.dll imports) are copied from this machine's Visual Studio redist
    folder. Not hash-pinned (they change with every VS update); instead each must carry a valid
    Microsoft Authenticode signature and be at least 14.44, the toolset onnxruntime.dll was linked
    with ("The version of the Redistributable ... must be the same or later than the version of
    the MSVC Build Tools used to create your application", Microsoft Learn).
  - The licence and notice files of both packages go to karaoke-runtime\licenses\.

  Idempotent: downloads are cached in .karaoke-runtime-cache\ (also gitignored) and files that
  already match are left alone. Run it once before `cargo tauri dev/build` on Windows; CI runs it
  in windows-release.yml and checks.yml.

  Keep the pinned values in sync with crates/karaoke/src/runtime.rs (a unit test compares them).
#>
[CmdletBinding()]
param(
    # Default: src-tauri\karaoke-runtime next to this script's repo.
    [string]$OutDir = '',
    # Default: .karaoke-runtime-cache at the repo root.
    [string]$CacheDir = '',
    # A Visual Studio "...\VC\Redist\MSVC\<version>" folder to take the VC++ DLLs from. Found with
    # vswhere when not given.
    [string]$VcRedistDir = ''
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version 2

# Windows PowerShell 5.1 leaves $PSScriptRoot empty inside param() defaults, so resolve them here.
$RepoRoot = Split-Path -Parent $PSScriptRoot
if (-not $OutDir) { $OutDir = Join-Path $RepoRoot 'src-tauri\karaoke-runtime' }
if (-not $CacheDir) { $CacheDir = Join-Path $RepoRoot '.karaoke-runtime-cache' }

# --- pinned ------------------------------------------------------------------------------------

$Packages = @(
    @{
        Name   = 'Microsoft.ML.OnnxRuntime.DirectML 1.24.4'
        Url    = 'https://api.nuget.org/v3-flatcontainer/microsoft.ml.onnxruntime.directml/1.24.4/microsoft.ml.onnxruntime.directml.1.24.4.nupkg'
        Sha256 = '57E9F11B73437BEF7A309496135D4C1F96B1A8E9DDBA60013FA27BFC1D788681'
        Files  = @(
            @{ Entry = 'runtimes/win-x64/native/onnxruntime.dll'; Dest = 'onnxruntime.dll'; Sha256 = 'E7EEDEC6A6F26DC39DC948276A75EF6D2BEE3FFF944D874CEED0BBD3B97BFF40' },
            @{ Entry = 'LICENSE'; Dest = 'licenses\onnxruntime-LICENSE.txt'; Sha256 = $null },
            @{ Entry = 'ThirdPartyNotices.txt'; Dest = 'licenses\onnxruntime-ThirdPartyNotices.txt'; Sha256 = $null }
        )
    },
    @{
        Name   = 'Microsoft.AI.DirectML 1.15.4'
        Url    = 'https://api.nuget.org/v3-flatcontainer/microsoft.ai.directml/1.15.4/microsoft.ai.directml.1.15.4.nupkg'
        Sha256 = '4E7CB7DDCE8CF837A7A75DC029209B520CA0101470FCDF275C1F49736A3615B9'
        Files  = @(
            @{ Entry = 'bin/x64-win/DirectML.dll'; Dest = 'DirectML.dll'; Sha256 = '9C9E6D822561C6C41B90E6994B3E8857CF1D66DBFB1E0C4C799C7C89B4E92DA1' },
            @{ Entry = 'LICENSE.txt'; Dest = 'licenses\DirectML-LICENSE.txt'; Sha256 = $null },
            @{ Entry = 'LICENSE-CODE.txt'; Dest = 'licenses\DirectML-LICENSE-CODE.txt'; Sha256 = $null },
            @{ Entry = 'ThirdPartyNotices.txt'; Dest = 'licenses\DirectML-ThirdPartyNotices.txt'; Sha256 = $null }
        )
    }
)
$VcDlls = @('vcruntime140.dll', 'vcruntime140_1.dll', 'msvcp140.dll', 'msvcp140_1.dll')
$VcMinVersion = [version]'14.44'

# --- helpers -----------------------------------------------------------------------------------

function Get-Sha256([string]$Path) {
    (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToUpperInvariant()
}

function Get-FileVersion([string]$Path) {
    $vi = (Get-Item -LiteralPath $Path).VersionInfo
    [version]::new($vi.FileMajorPart, $vi.FileMinorPart, $vi.FileBuildPart, $vi.FilePrivatePart)
}

function Get-Package($Pkg) {
    New-Item -ItemType Directory -Force -Path $CacheDir | Out-Null
    $file = Join-Path $CacheDir ([IO.Path]::GetFileName($Pkg.Url))
    if ((Test-Path -LiteralPath $file) -and (Get-Sha256 $file) -eq $Pkg.Sha256) {
        Write-Host "cached   $($Pkg.Name)"
        return $file
    }
    Write-Host "download $($Pkg.Name)"
    $part = "$file.part"
    & curl.exe -fsSL --retry 3 -o $part $Pkg.Url
    if ($LASTEXITCODE -ne 0) { throw "download failed: $($Pkg.Url)" }
    $got = Get-Sha256 $part
    if ($got -ne $Pkg.Sha256) {
        Remove-Item -LiteralPath $part -Force
        throw "$($Pkg.Name): SHA-256 $got does not match the pinned $($Pkg.Sha256)"
    }
    Move-Item -LiteralPath $part -Destination $file -Force
    return $file
}

function Find-VcCrtDir {
    $roots = @()
    if ($VcRedistDir) { $roots += $VcRedistDir }
    elseif ($env:VCToolsRedistDir) { $roots += $env:VCToolsRedistDir }
    else {
        $vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
        if (-not (Test-Path -LiteralPath $vswhere)) { throw 'vswhere.exe not found: install Visual Studio (or Build Tools) with the C++ workload, or pass -VcRedistDir' }
        $vs = & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
        if (-not $vs) { throw 'no Visual Studio with the C++ x64 tools found; pass -VcRedistDir' }
        $msvc = Join-Path $vs 'VC\Redist\MSVC'
        # Newest toolset first; the 14.xx.yyyyy folders, not the vNNN aliases.
        $roots += Get-ChildItem -LiteralPath $msvc -Directory |
            Where-Object { $_.Name -match '^\d+\.\d+\.\d+$' } |
            Sort-Object { [version]$_.Name } -Descending |
            ForEach-Object { $_.FullName }
    }
    foreach ($root in $roots) {
        $crt = Get-ChildItem -LiteralPath (Join-Path $root 'x64') -Directory -Filter 'Microsoft.VC*.CRT' -ErrorAction SilentlyContinue |
            Select-Object -First 1
        if (-not $crt) { continue }
        $all = $true
        foreach ($dll in $VcDlls) {
            $p = Join-Path $crt.FullName $dll
            if (-not (Test-Path -LiteralPath $p) -or (Get-FileVersion $p) -lt $VcMinVersion) { $all = $false; break }
        }
        if ($all) { return $crt.FullName }
    }
    throw "no VC++ redist folder with $($VcDlls -join ', ') at version >= $VcMinVersion (looked in: $($roots -join '; '))"
}

function Assert-MicrosoftSigned([string]$Path) {
    $sig = Get-AuthenticodeSignature -LiteralPath $Path
    if ($sig.Status -ne 'Valid') { throw "$Path`: Authenticode signature is $($sig.Status)" }
    if ($sig.SignerCertificate.Subject -notmatch 'O=Microsoft Corporation') {
        throw "$Path`: signed by '$($sig.SignerCertificate.Subject)', not Microsoft Corporation"
    }
}

# --- run ---------------------------------------------------------------------------------------

Add-Type -AssemblyName System.IO.Compression.FileSystem
New-Item -ItemType Directory -Force -Path (Join-Path $OutDir 'licenses') | Out-Null
$OutDir = (Resolve-Path -LiteralPath $OutDir).Path

foreach ($pkg in $Packages) {
    $nupkg = $null
    foreach ($f in $pkg.Files) {
        $dest = Join-Path $OutDir $f.Dest
        if ($f.Sha256 -and (Test-Path -LiteralPath $dest) -and (Get-Sha256 $dest) -eq $f.Sha256) { continue }
        if (-not $nupkg) { $nupkg = Get-Package $pkg }
        $zip = [IO.Compression.ZipFile]::OpenRead($nupkg)
        try {
            $entry = $zip.Entries | Where-Object { $_.FullName -eq $f.Entry } | Select-Object -First 1
            if (-not $entry) { throw "$($pkg.Name): no '$($f.Entry)' in the package" }
            [IO.Compression.ZipFileExtensions]::ExtractToFile($entry, $dest, $true)
        }
        finally { $zip.Dispose() }
        if ($f.Sha256) {
            $got = Get-Sha256 $dest
            if ($got -ne $f.Sha256) {
                Remove-Item -LiteralPath $dest -Force
                throw "$($f.Dest): SHA-256 $got does not match the pinned $($f.Sha256)"
            }
        }
    }
}

$crt = Find-VcCrtDir
foreach ($dll in $VcDlls) {
    $src = Join-Path $crt $dll
    Assert-MicrosoftSigned $src
    Copy-Item -LiteralPath $src -Destination (Join-Path $OutDir $dll) -Force
}

Write-Host ''
Write-Host "karaoke runtime in $OutDir (VC++ DLLs from $crt):"
Get-ChildItem -LiteralPath $OutDir -File | Sort-Object Name | ForEach-Object {
    $v = $_.VersionInfo
    '  {0,-20} {1,10:N0} B  {2,-16} {3}' -f $_.Name, $_.Length, ("$($v.FileMajorPart).$($v.FileMinorPart).$($v.FileBuildPart)"), (Get-Sha256 $_.FullName)
}
