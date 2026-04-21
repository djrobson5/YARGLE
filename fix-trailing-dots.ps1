param(
    [Parameter(Mandatory=$true, Position=0)]
    [string]$Root,

    [switch]$Execute
)

$ErrorActionPreference = 'Stop'

function To-LongPath([string]$p) {
    if ($p.StartsWith('\\?\')) { return $p }
    return '\\?\' + ($p -replace '/', '\')
}

$resolved = (Resolve-Path -LiteralPath $Root).Path
$rootLong = To-LongPath $resolved

Write-Host "Scanning: $resolved"
if ($Execute) {
    Write-Host "Mode:     EXECUTE (renames will be applied)"
} else {
    Write-Host "Mode:     DRY RUN (re-run with -Execute to apply)"
}
Write-Host ""

$entries = [System.IO.Directory]::EnumerateFileSystemEntries(
    $rootLong, '*', [System.IO.SearchOption]::AllDirectories
) | ForEach-Object {
    $full = $_
    $name = [System.IO.Path]::GetFileName($full)
    if ($name -match '[. ]+$') {
        [PSCustomObject]@{
            Full    = $full
            Name    = $name
            Trimmed = $name.TrimEnd('.', ' ')
            IsDir   = [System.IO.Directory]::Exists($full)
            Depth   = ($full -split '\\').Count
        }
    }
} | Sort-Object -Property Depth -Descending

if (-not $entries) {
    Write-Host "No entries with trailing dots or spaces found."
    return
}

$renamed = 0
$skipped = 0

foreach ($e in $entries) {
    $parent  = [System.IO.Path]::GetDirectoryName($e.Full)
    $newFull = [System.IO.Path]::Combine($parent, $e.Trimmed)

    if ([string]::IsNullOrWhiteSpace($e.Trimmed)) {
        Write-Warning "Skip (empty after trim): $($e.Full)"
        $skipped++
        continue
    }

    if ([System.IO.File]::Exists($newFull) -or [System.IO.Directory]::Exists($newFull)) {
        Write-Warning "Skip (target exists):   $newFull"
        $skipped++
        continue
    }

    $kind = if ($e.IsDir) { 'dir ' } else { 'file' }
    # Strip the \\?\ prefix for display
    $displayOld = $e.Full    -replace '^\\\\\?\\', ''
    $displayNew = $newFull   -replace '^\\\\\?\\', ''
    Write-Host "$kind  $displayOld"
    Write-Host "   -> $displayNew"

    if ($Execute) {
        # Two-step rename via a unique temp name. Direct rename between names
        # that differ only in trailing dots/spaces can be normalized away by
        # the OS even through \\?\ on some Windows builds.
        $tmp = [System.IO.Path]::Combine(
            $parent,
            "__yargle_tmp_" + [System.Guid]::NewGuid().ToString("N")
        )
        try {
            if ($e.IsDir) {
                [System.IO.Directory]::Move($e.Full, $tmp)
                [System.IO.Directory]::Move($tmp, $newFull)
            } else {
                [System.IO.File]::Move($e.Full, $tmp)
                [System.IO.File]::Move($tmp, $newFull)
            }
            $renamed++
        } catch {
            Write-Warning "Failed: $($_.Exception.Message)"
            $skipped++
        }
    } else {
        $renamed++
    }
}

Write-Host ""
if ($Execute) {
    Write-Host "Done. Renamed: $renamed  Skipped: $skipped"
} else {
    Write-Host "Would rename: $renamed  Would skip: $skipped"
    Write-Host "Re-run with -Execute to apply."
}
