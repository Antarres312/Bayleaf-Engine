# Runs the physics benchmark (bench/physics) one case at a time and appends its JSON lines to
# bench/results/<Name>.jsonl, each with the label, the threads asked for, the run and how busy the
# processors were just before it. See bench/README.md, "Physics on several cores".
#
# Run it from PowerShell, or with -Command (powershell -Command "& .\bench\run-physics.ps1 ..."):
# through -File a list such as -Threads 0,1,8 arrives as one number.
#
# -Label names the build being measured (for example "before" for a build without Rapier's
# parallel feature). -Threads lists the thread counts to run (0: the default); a build without
# the parallel feature takes only 0.
param(
    [int[]]$Bodies = @(5000, 10000, 20000),
    [int[]]$Threads = @(0),
    [int]$Repeats = 3,
    [string]$Label = 'after',
    [string]$Precision = 'single',
    [int]$Steps = 240,
    # Each case starts once the processors are less busy than this (percent, all of them together).
    [double]$QuietPercent = 15,
    # ...or once it has waited this long (the load is recorded with each run either way).
    [double]$WaitMinutes = 3,
    [string]$Exe = '',
    [string]$Name = ((Get-Date -Format 'yyyy-MM-dd') + '-physics'),
    # Only rewrites the summary (<Name>.md) from the runs already in <Name>.jsonl.
    [switch]$SummaryOnly
)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
if (-not $Exe) {
    $target = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $root 'target' }
    $Exe = Join-Path $target 'release\bayleaf-bench-physics.exe'
}
$results = Join-Path $PSScriptRoot 'results'
New-Item -ItemType Directory -Force $results | Out-Null
$out = Join-Path $results "$Name.jsonl"

# Waits (up to -WaitMinutes) until the processors are mostly idle; returns how busy they were.
function Wait-Quiet {
    $deadline = (Get-Date).AddMinutes($WaitMinutes)
    while ($true) {
        $busy = ((Get-Counter '\Processor(_Total)\% Processor Time' -SampleInterval 2 -MaxSamples 2).CounterSamples | Measure-Object CookedValue -Average).Average
        if ($busy -lt $QuietPercent -or (Get-Date) -gt $deadline) { return [math]::Round($busy, 1) }
        Write-Host "    waiting: processors $([math]::Round($busy)) % busy"
    }
}

# Appends one line to the results, trying again while another program has the file open.
function Add-Line([string]$text) {
    for ($try = 1; ; $try++) {
        try { [IO.File]::AppendAllText($out, $text + [Environment]::NewLine); return }
        catch { if ($try -ge 20) { throw } else { Start-Sleep -Milliseconds 500 } }
    }
}

# Every case, run after run, so a slow spell on the computer does not land on one case only.
function Invoke-All {
    for ($r = 1; $r -le $Repeats; $r++) {
        foreach ($n in $Bodies) {
            foreach ($t in $Threads) {
                $busy = Wait-Quiet
                Write-Host "$Label, $n bodies, threads $t, run $r of $Repeats (processors $busy % busy)"
                $caseArgs = @('--bodies', "$n", '--steps', "$Steps", '--precision', $Precision)
                if ($t -gt 0) { $caseArgs += @('--threads', "$t") }
                $line = & $Exe @caseArgs
                if ($LASTEXITCODE -ne 0) { throw "the case failed: $Exe $caseArgs" }
                $case = $line | ConvertFrom-Json
                $case | Add-Member label $Label
                $case | Add-Member asked_threads $t
                $case | Add-Member run $r
                $case | Add-Member busy_before $busy
                Add-Line ($case | ConvertTo-Json -Compress)
            }
        }
    }
}

# One row per build, thread count asked for and size: the median of the runs' means, their range, the
# median of the runs' medians, how busy the processors were, and whether every run of that size
# (in every build and thread count) ended with the same checksum.
function Write-Summary {
    $cases = Get-Content $out | ForEach-Object { $_ | ConvertFrom-Json }
    $median = { param($xs) $s = @($xs | Sort-Object); if ($s.Count % 2) { $s[($s.Count - 1) / 2] } else { ($s[$s.Count / 2 - 1] + $s[$s.Count / 2]) / 2 } }
    $lines = @("# Physics step, $Name", '', "Milliseconds per step; each row is the median over its runs. Raw runs: $Name.jsonl.", '',
        '| Build | Threads | Bodies | Precision | Runs | Mean | Means, range | Median | Busy before, % | Same result |',
        '|---|---:|---:|---|---:|---:|---|---:|---|---|')
    $groups = $cases | Group-Object label, asked_threads, bodies, precision
    foreach ($g in $groups) {
        $first = $g.Group[0]
        $means = $g.Group | ForEach-Object { [double]$_.mean_ms }
        $sums = @($cases | Where-Object { $_.bodies -eq $first.bodies -and $_.precision -eq $first.precision } | ForEach-Object checksum | Sort-Object -Unique)
        # A build without the parallel feature reports no threads: it steps on one.
        $threads = if ($null -eq $first.threads) { "1, serial build" } elseif ($first.asked_threads -eq 0) { "default ($($first.threads))" } else { $first.threads }
        $busy = ($g.Group | ForEach-Object { ([double]$_.busy_before).ToString([cultureinfo]::InvariantCulture) }) -join ', '
        $lines += "| $($first.label) | $threads | $($first.bodies) | $($first.precision) | $($g.Count) | $([math]::Round((& $median $means), 2)) | $([math]::Round(($means | Measure-Object -Minimum).Minimum, 2)) to $([math]::Round(($means | Measure-Object -Maximum).Maximum, 2)) | $([math]::Round((& $median ($g.Group | ForEach-Object { [double]$_.median_ms })), 2)) | $busy | $(if ($sums.Count -eq 1) { 'yes' } else { "no ($($sums.Count) checksums)" }) |"
    }
    [IO.File]::WriteAllLines((Join-Path $results "$Name.md"), [string[]]$lines)
    $lines | Write-Host
}

if (-not $SummaryOnly) { Invoke-All }
Write-Summary
