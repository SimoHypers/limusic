$ErrorActionPreference = "Continue"
Set-Location "C:\Users\Administrator\Documents\Limusic"
$p = "ui/src/routes/playlist/[id]/+page.svelte"

Write-Output "== 1) is the reviewer's requested reset COMMITTED in HEAD blob? =="
$inHead = git show "HEAD:$p" | Select-String -Pattern "mounted = MOUNT_BAND;"
if ($inHead) {
  Write-Output ("  YES: HEAD blob line " + $inHead.LineNumber + " -> " + $inHead.Line.Trim())
} else {
  Write-Output "  NO — not in HEAD yet; committing it now."
  git add -- "$p"
  git commit -m "fix: reset the mount band when a playlist is swapped in load(pid)" `
             -m "New playlist, fresh band. `shown` is recomputed when `pl` lands below; leaving mounted at the previous list's (possibly tens-of-thousands) row count would make the new one render in full through Math.min(mounted, shown.length) instead of band-by-band. Reset before `pl` is replaced — after it, shown refers to the new list and this is a no-op that never runs again."
}

Write-Output ""
Write-Output "== 2) confirm working tree is now CLEAN for that file and overall =="
git status --short

Write-Output ""
Write-Output "== 3) sanity: the net PR diff vs origin/master still carries ONLY the semantic fixes =="
git diff origin/master --stat

Write-Output ""
Write-Output "== 4) cleanup: remove temp/scratch files I created during diagnostics (NOT project files) =="
$scratch = @(
  "scripts\verify-fix-encoding.ps1",
  "scripts\verify-fix-encoding-disk.ps1",
  "scripts\verify-fix-encoding-src.ps1",
  "scripts\rc-one-file.ps1",
  "scripts\rc-one-file-clean.ps1",
  "scripts\encoding-http.ps1",
  "scripts\encoding-http2.ps1",
  "scripts\verify-fix-encoding2.ps1",
  "scripts\fix-encoding-disk.ps1",
  "scripts\verify-fix-encoding.ps1.tmp",
  "scripts\normalize-wiki.ps1",
  "scripts\utf8-check.ps1",
  "src-tauri\tauri-live-build.log",
  "src-tauri\tauri-build.log",
  "src-tauri\tauri-build2.log",
  "src-tauri\tauri-build-override.json",
  "src-tauri\opencode-build.json",
  "src-tauri\build-override.json",
  "src-tauri\tauri.conf.bak.json",
  "src-tauri\tauri-live-build2.log",
  "encoding_check.ps1",
  "verify-fix-encoding.ps1"
)
foreach ($f in $scratch) {
  if (Test-Path -LiteralPath $f) { Remove-Item -LiteralPath $f -Force; Write-Output "  removed $f" }
}
Write-Output ""
Write-Output "== 5) final: git status + last 2 commits =="
git status --short
git log --oneline -3
