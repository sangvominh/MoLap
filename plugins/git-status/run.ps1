param([switch]$Fetch)
$ErrorActionPreference = 'SilentlyContinue'

$repo = $env:MOLAP_GIT_DIR
if (-not $repo) {
    $candidate = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
    if (Test-Path (Join-Path $candidate '.git')) {
        $repo = $candidate
    }
}

if (-not $repo -or -not (Get-Command git -ErrorAction SilentlyContinue)) {
    Write-Output 'title=Git Status'
    Write-Output 'badge=IDLE'
    Write-Output 'tone=normal'
    Write-Output 'value=No Repository'
    Write-Output 'subtitle=Set MOLAP_GIT_DIR or run inside a git repo'
    Write-Output 'alert=false'
    exit 0
}

if ($Fetch) {
    git -C $repo fetch --quiet 2>$null
}

$branch = (git -C $repo branch --show-current 2>$null).Trim()
if (-not $branch) {
    $branch = (git -C $repo rev-parse --short HEAD 2>$null).Trim()
}
$dirty = @(git -C $repo status --porcelain 2>$null).Count
$lastCommit = (git -C $repo log -1 --pretty=%s 2>$null).Trim()
if (-not $lastCommit) { $lastCommit = 'No commits yet' }
$aheadBehind = (git -C $repo rev-list --left-right --count '@{u}...HEAD' 2>$null).Trim()

$badge = if ($dirty -gt 0) { "$dirty DIRTY" } else { 'CLEAN' }
$tone = if ($dirty -gt 0) { 'warn' } else { 'good' }
$repoName = Split-Path $repo -Leaf

Write-Output 'title=Git Status'
Write-Output ('badge={0}' -f $badge)
Write-Output ('tone={0}' -f $tone)
Write-Output ('value={0}' -f $branch)
Write-Output ('subtitle={0} · {1}' -f $repoName, $lastCommit)
Write-Output ('pill={0} · {1}' -f $branch, $badge)
Write-Output 'alert=false'
Write-Output ('item=Changed Files | {0}' -f $dirty)
if ($aheadBehind -match '^(\d+)\s+(\d+)$') {
    Write-Output ('item=Sync | +{0} / -{1}' -f $Matches[2], $Matches[1])
} else {
    Write-Output 'item=Sync | Local branch'
}
Write-Output ('item=Repository | {0}' -f $repoName)
