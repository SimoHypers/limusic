Set-StrictMode -Version 2.0
Set-Location "C:\Users\Administrator\Documents\Limusic"
$p = "ui/src/routes/playlist/[id]/+page.svelte"

Write-Output "== does the reset line exist on disk? =="
$hit = Select-String -LiteralPath $p -SimpleMatch -Pattern "mounted = MOUNT_BAND;"
if ($hit) { Write-Output ("  DISK line {0}: {1}" -f $hit.LineNumber, $hit.Line.Trim()) } else { Write-Output "  DISK: NOT FOUND" }

Write-Output "== is it already committed (in HEAD blob)? =="
$blob = git show "HEAD:$p"
$chit = $blob | Select-String -SimpleMatch -Pattern "mounted = MOUNT_BAND;"
if ($chit) { Write-Output ("  HEAD blob line {0}: {1}" -f $chit.LineNumber, $chit.Line.Trim()) } else { Write-Output "  HEAD blob: NOT YET committed" }

Write-Output "== uncommitted diff for this file? =="
$d = git diff HEAD -- $p
if ([string]::IsNullOrEmpty($d)) { Write-Output "  (none — file matches HEAD)" } else { Write-Output $d }

Write-Output "== anything left uncommitted across the repo right now? =="
git status --short
