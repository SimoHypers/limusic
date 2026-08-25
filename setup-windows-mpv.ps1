$ErrorActionPreference = "Stop"

$workspace = Get-Location
$mpvDir = Join-Path $workspace ".libmpv"
$srcTauriDir = Join-Path $workspace "src-tauri"

Write-Host "Setting up libmpv in $mpvDir..."
New-Item -ItemType Directory -Force -Path $mpvDir | Out-Null
New-Item -ItemType Directory -Force -Path $srcTauriDir | Out-Null

$zipPath = Join-Path $mpvDir "mpv-dev.7z"
$dllPath = Join-Path $mpvDir "libmpv-2.dll"
$shaMarker = Join-Path $mpvDir "mpv-dev.sha256"
$url = "https://github.com/shinchiro/mpv-winbuild-cmake/releases/download/20260610/mpv-dev-x86_64-20260610-git-304426c.7z"
# The GitHub release asset's own `digest`. It pins $url, so moving one without the other fails the
# check rather than skipping it. windows-release.yml carries the same pair as MPV_DEV_SHA256.
$expectedSha = "8cbb25ea784f01afbb3f904217cab1317430a8bcfd5680fd827a866367f71cc9"

# A leftover DLL from an earlier $url would otherwise skip the digest check entirely, so the marker
# records which archive produced the files rather than trusting that one of them exists. It holds the
# URL too: bumping $url and forgetting $expectedSha would otherwise still match and reuse the old DLL.
# `-as [string]` rather than a cast, because Get-Content on a 0-byte file yields $null, not "".
$markerValue = "$url|$expectedSha"
$verified = (Test-Path $dllPath) -and (Test-Path $shaMarker) -and
    ((((Get-Content $shaMarker -Raw) -as [string]).Trim()) -eq $markerValue)

if (-not $verified) {
    Write-Host "Downloading mpv dev package..."
    curl.exe -fSL --retry 3 -o $zipPath $url
    # A native exe's non-zero exit does not trip $ErrorActionPreference.
    if ($LASTEXITCODE -ne 0) {
        Remove-Item $zipPath -ErrorAction SilentlyContinue
        throw "curl exited $LASTEXITCODE fetching $url"
    }
    $actualSha = (Get-FileHash $zipPath -Algorithm SHA256).Hash
    if ($actualSha -ne $expectedSha) {
        Remove-Item $zipPath -ErrorAction SilentlyContinue
        throw "mpv-dev.7z SHA-256 is $actualSha, expected $expectedSha. Not extracting it."
    }
    # Cleared only now that the archive is in hand and verified. An archive that turns out not to
    # carry the DLL would otherwise leave the previous one in place for the marker to vouch for.
    Remove-Item $dllPath -ErrorAction SilentlyContinue
    Remove-Item $shaMarker -ErrorAction SilentlyContinue
    Write-Host "Extracting..."
    tar.exe -xf $zipPath -C $mpvDir
    $tarExit = $LASTEXITCODE
    Remove-Item $zipPath -ErrorAction SilentlyContinue
    if ($tarExit -ne 0) { throw "tar exited $tarExit extracting mpv-dev.7z" }
    if (-not (Test-Path $dllPath)) { throw "mpv-dev.7z did not contain libmpv-2.dll" }
    Set-Content -Path $shaMarker -Value $markerValue -Encoding ascii -Force
}

if (-not (Test-Path $dllPath)) {
    throw "libmpv-2.dll not found in $mpvDir"
}

# Copy DLL to src-tauri so Tauri bundle includes it
Copy-Item $dllPath (Join-Path $srcTauriDir "libmpv-2.dll") -Force
Write-Host "Copied libmpv-2.dll to src-tauri/libmpv-2.dll"

# Locate MSVC toolchain for dumpbin and lib.exe
$msvcBin = ""
$dumpbin = Get-Command "dumpbin.exe" -ErrorAction SilentlyContinue
$libExe = Get-Command "lib.exe" -ErrorAction SilentlyContinue

if ($dumpbin -and $libExe) {
    $dumpbinPath = $dumpbin.Source
    $libPath = $libExe.Source
} else {
    $msvcTools = Get-ChildItem "C:\Program Files (x86)\Microsoft Visual Studio" -Recurse -Filter "dumpbin.exe" -ErrorAction SilentlyContinue | Select-Object -First 1
    if ($msvcTools) {
        $msvcBin = $msvcTools.DirectoryName
        $dumpbinPath = Join-Path $msvcBin "dumpbin.exe"
        $libPath = Join-Path $msvcBin "lib.exe"
    } else {
        throw "Could not find Visual Studio MSVC tools (dumpbin.exe / lib.exe)"
    }
}

Write-Host "Using dumpbin at: $dumpbinPath"
Write-Host "Using lib at: $libPath"

# Absolute paths throughout: a Set-Location here outlives the script and would strand the caller
# in .libmpv on any throw below.
$defPath = Join-Path $mpvDir "mpv.def"
$libOut = Join-Path $mpvDir "mpv.lib"
$exports = & $dumpbinPath /exports $dllPath |
    Select-String -Pattern '^\s+\d+\s+[0-9A-Fa-f]+\s+[0-9A-Fa-f]+\s+(\w+)' |
    ForEach-Object { $_.Matches[0].Groups[1].Value }

if ($exports.Count -lt 50) {
    throw "Parsed only $($exports.Count) exports from libmpv-2.dll"
}

@("EXPORTS") + ($exports | ForEach-Object { "    $_" }) | Set-Content -Path $defPath -Encoding ascii
# Cleared first, so a failed lib.exe cannot leave last run's mpv.lib to pass the check below.
Remove-Item $libOut -ErrorAction SilentlyContinue
& $libPath "/def:$defPath" /name:libmpv-2.dll "/out:$libOut" /machine:x64
if ($LASTEXITCODE -ne 0) { throw "lib.exe exited $LASTEXITCODE generating mpv.lib" }

if (-not (Test-Path $libOut)) {
    throw "Failed to generate mpv.lib"
}

Write-Host "Successfully generated mpv.lib ($($exports.Count) exports)"

# Ensure .cargo/config.toml exists so cargo automatically finds mpv.lib
$cargoDir = Join-Path $workspace ".cargo"
New-Item -ItemType Directory -Force -Path $cargoDir | Out-Null
$cargoConfig = Join-Path $cargoDir "config.toml"

$escapedMpvDir = $mpvDir -replace '\\', '/'
$configContent = @"
[build]
rustflags = ["-L", "native=$escapedMpvDir"]
"@

$configNote = $null
if (-not (Test-Path $cargoConfig)) {
    Set-Content -Path $cargoConfig -Value $configContent -Encoding utf8
    Write-Host "Configured .cargo/config.toml with rustflags"
} else {
    # Whether an existing config already links mpv cannot be decided without a TOML parser: the path
    # may be a basic or a literal string, the flag may sit under [build] or a [target.*] table, and
    # RUSTFLAGS in the environment overrides both. Overwriting it would drop settings this script
    # knows nothing about, so it is reported and left alone rather than edited or guessed at.
    $configNote = "$cargoConfig already existed and was left untouched. Check that it, or `$env:RUSTFLAGS, passes -L native=$escapedMpvDir to rustc."
}

if ($configNote) {
    Write-Host ""
    Write-Host "Setup finished. mpv.lib is built, but one thing needs your eyes:"
    Write-Host "  $configNote"
} else {
    Write-Host "Setup completed successfully!"
}
