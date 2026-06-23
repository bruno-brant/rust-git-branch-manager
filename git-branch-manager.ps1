#!/usr/bin/env pwsh
# Interactive git branch manager.
# Navigate with ↑/↓, Space to select, Enter to delete, r to refresh, q to quit.

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

# ─── Git helpers ────────────────────────────────────────────────────────────

function Get-WorktreeMap {
    # Returns @{ branch-name = worktree-path } for linked worktrees only.
    $map    = @{}
    $isFirst = $true
    $path   = $null
    git worktree list --porcelain 2>$null | ForEach-Object {
        if ($_ -match '^worktree (.+)$') {
            $path = $Matches[1]
        } elseif ($_ -eq '') {
            $isFirst = $false   # blank line separates entries; first entry is main
        } elseif (-not $isFirst -and $_ -match '^branch refs/heads/(.+)$') {
            $map[$Matches[1]] = $path
        }
    }
    $map
}

function Get-GitBranches {
    $mergedSet = [System.Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
    git branch --merged HEAD 2>$null |
        ForEach-Object { $mergedSet.Add(($_ -replace '^\*?\s+', '')) | Out-Null }

    $wt = Get-WorktreeMap

    @(git branch --format='%(refname:short)|%(HEAD)' 2>$null |
        ForEach-Object {
            $parts  = $_ -split '\|', 2
            $name   = $parts[0]
            $isHead = $parts.Count -gt 1 -and $parts[1] -eq '*'
            [PSCustomObject]@{
                Name     = $name
                IsHead   = $isHead
                IsMerged = $mergedSet.Contains($name)
                Worktree = if ($wt.ContainsKey($name)) { $wt[$name] } else { $null }
                Selected = $false
            }
        })
}

function Invoke-GitDelete {
    param([string]$Name, [bool]$Force)
    $flag = if ($Force) { '-D' } else { '-d' }
    $out  = git branch $flag $Name 2>&1
    if ($LASTEXITCODE -ne 0) { throw $out }
}

function Invoke-RemoveWorktreeAndBranch {
    param([string]$Name, [string]$WorktreePath)
    $out = git worktree remove --force $WorktreePath 2>&1
    if ($LASTEXITCODE -ne 0) { throw $out }
    Invoke-GitDelete -Name $Name -Force $false
}

# ─── Rendering ──────────────────────────────────────────────────────────────

function Get-ConsoleWidth { [Math]::Max(40, [Console]::WindowWidth) }

function Write-ColorLine {
    param([string]$Text, $Fg = $null, $Bg = $null)
    $w    = Get-ConsoleWidth
    $line = $Text.PadRight($w)
    if ($line.Length -gt $w) { $line = $line.Substring(0, $w) }
    if ($Fg) { [Console]::ForegroundColor = $Fg }
    if ($Bg) { [Console]::BackgroundColor = $Bg }
    [Console]::Write($line)
    [Console]::ResetColor()
    [Console]::WriteLine()
}

function Build-BranchRow {
    param([PSCustomObject]$Branch, [bool]$IsCursor)
    $cursor = if ($IsCursor) { '>' } else { ' ' }
    $head   = if ($Branch.IsHead) { '*' } else { ' ' }
    $box    = if ($Branch.Selected) { '[x]' } else { '[ ]' }
    $suffix = ''
    if ($Branch.Worktree) { $suffix += "  (wt: $($Branch.Worktree))" }
    if (-not $Branch.IsMerged -and -not $Branch.IsHead) { $suffix += '  (unmerged - needs force)' }
    "$cursor $head $box $($Branch.Name)$suffix"
}

function Draw-Screen {
    param(
        [object[]]$Branches,
        [int]$Cursor,
        [int]$Offset,
        [string]$Status
    )

    $viewH = [Math]::Max(1, [Console]::WindowHeight - 3)  # minus title, status, help

    [Console]::SetCursorPosition(0, 0)

    # Title
    $pos   = if ($Branches.Count -eq 0) { 0 } else { $Cursor + 1 }
    Write-ColorLine " git-branch-manager   $pos/$($Branches.Count)" -Fg White -Bg DarkBlue

    # Branch rows
    $end = [Math]::Min($Offset + $viewH, $Branches.Count)
    for ($i = $Offset; $i -lt ($Offset + $viewH); $i++) {
        if ($i -lt $Branches.Count) {
            $b    = $Branches[$i]
            $row  = Build-BranchRow -Branch $b -IsCursor ($i -eq $Cursor)
            $fg   = if ($b.IsHead) { 'DarkGreen' } elseif ($b.Selected) { 'Cyan' } else { $null }
            if ($i -eq $Cursor) {
                Write-ColorLine $row -Fg White -Bg DarkCyan
            } else {
                Write-ColorLine $row -Fg $fg
            }
        } else {
            Write-ColorLine ''
        }
    }

    # Status line
    Write-ColorLine $Status -Fg Magenta

    # Help line
    Write-ColorLine '  /  navigate   Space select   Enter delete   r refresh   q quit' -Fg DarkGray
}

function Clamp-Offset {
    param([int]$Cursor, [int]$Offset, [int]$ViewH, [int]$Total)
    if ($Cursor -lt $Offset) { $Offset = $Cursor }
    elseif ($Cursor -ge ($Offset + $ViewH)) { $Offset = $Cursor - $ViewH + 1 }
    $max = [Math]::Max(0, $Total - $ViewH)
    [Math]::Min($Offset, $max)
}

# ─── Deletion logic ─────────────────────────────────────────────────────────

function Invoke-DeleteTargets {
    param([object[]]$Branches, [object[]]$Targets, [int]$BottomRow)

    $unmerged = @($Targets | Where-Object { -not $_.IsMerged -and -not $_.IsHead })
    $withWt   = @($Targets | Where-Object { $_.Worktree })

    if ($unmerged.Count -gt 0 -or $withWt.Count -gt 0) {
        # Inline confirmation at the bottom of the terminal
        $names = ($unmerged + $withWt | Select-Object -ExpandProperty Name -Unique) -join ', '
        [Console]::SetCursorPosition(0, $BottomRow)
        [Console]::ForegroundColor = 'Yellow'
        $prompt = "  Force/worktree delete: $names  [y/N] "
        [Console]::Write($prompt.PadRight((Get-ConsoleWidth)))
        [Console]::SetCursorPosition($prompt.Length, $BottomRow)
        [Console]::CursorVisible = $true
        [Console]::ResetColor()
        $ans = [Console]::ReadKey($true)
        [Console]::CursorVisible = $false
        if ($ans.KeyChar -ne 'y' -and $ans.KeyChar -ne 'Y') {
            return 'cancelled'
        }
    }

    $deleted = 0
    $errors  = [System.Collections.Generic.List[string]]::new()

    foreach ($b in $Targets) {
        if ($b.IsHead) { continue }
        try {
            if ($b.Worktree) {
                Invoke-RemoveWorktreeAndBranch -Name $b.Name -WorktreePath $b.Worktree
            } else {
                Invoke-GitDelete -Name $b.Name -Force (-not $b.IsMerged)
            }
            $deleted++
        } catch {
            $errors.Add("$($b.Name): $_")
        }
    }

    $msg = "deleted $deleted branch(es)"
    if ($errors.Count -gt 0) { $msg += " — $($errors -join '; ')" }
    $msg
}

# ─── Main ───────────────────────────────────────────────────────────────────

if (-not (git rev-parse --is-inside-work-tree 2>$null)) {
    Write-Error "Not inside a git repository."
    exit 1
}

$branches = Get-GitBranches
$cursor   = 0
$offset   = 0
$status   = ''

if ($branches.Count -eq 0) {
    Write-Host 'No local branches found.'
    exit 0
}

[Console]::CursorVisible = $false
[Console]::Clear()

try {
    :main while ($true) {
        $viewH  = [Math]::Max(1, [Console]::WindowHeight - 3)
        $offset = Clamp-Offset -Cursor $cursor -Offset $offset -ViewH $viewH -Total $branches.Count

        Draw-Screen -Branches $branches -Cursor $cursor -Offset $offset -Status $status
        $status = ''

        $k = [Console]::ReadKey($true)

        # Ctrl+C
        if ($k.Key -eq [ConsoleKey]::C -and ($k.Modifiers -band [ConsoleModifiers]::Control)) {
            break main
        }

        switch ($k.Key) {
            ([ConsoleKey]::UpArrow)   { if ($cursor -gt 0) { $cursor-- } }
            ([ConsoleKey]::DownArrow) { if ($cursor -lt ($branches.Count - 1)) { $cursor++ } }
            ([ConsoleKey]::PageUp)    { $cursor = [Math]::Max(0, $cursor - $viewH) }
            ([ConsoleKey]::PageDown)  { $cursor = [Math]::Min($branches.Count - 1, $cursor + $viewH) }
            ([ConsoleKey]::Home)      { $cursor = 0 }
            ([ConsoleKey]::End)       { $cursor = $branches.Count - 1 }

            ([ConsoleKey]::Spacebar) {
                $b = $branches[$cursor]
                if ($b.IsHead) {
                    $status = "'$($b.Name)' is the current branch and cannot be deleted"
                } else {
                    $branches[$cursor].Selected = -not $branches[$cursor].Selected
                }
            }

            ([ConsoleKey]::Enter) {
                $targets = @($branches | Where-Object { $_.Selected })
                if ($targets.Count -eq 0) {
                    $b = $branches[$cursor]
                    if (-not $b.IsHead) { $targets = @($b) }
                }
                if ($targets.Count -eq 0) {
                    $status = 'nothing to delete — select branches with Space'
                } else {
                    $bottomRow = [Console]::WindowHeight - 1
                    $result    = Invoke-DeleteTargets -Branches $branches -Targets $targets -BottomRow $bottomRow
                    $status    = $result
                    $branches  = Get-GitBranches
                    if ($cursor -ge $branches.Count) { $cursor = [Math]::Max(0, $branches.Count - 1) }
                }
            }

            ([ConsoleKey]::R) {
                $branches = Get-GitBranches
                if ($cursor -ge $branches.Count) { $cursor = [Math]::Max(0, $branches.Count - 1) }
                $status = 'refreshed'
            }

            ([ConsoleKey]::Q)      { break main }
            ([ConsoleKey]::Escape) { break main }
        }
    }
} finally {
    [Console]::CursorVisible = $true
    [Console]::ResetColor()
    [Console]::Clear()
}
