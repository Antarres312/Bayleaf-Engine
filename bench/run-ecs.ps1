# The ECS on one thread and on the thread pool (bench/README.md, "The ECS on several cores"): builds
# the micro-benchmark (crates/ecs/examples/parallel.rs), runs every case -Repeats times, each in its
# own process once the processors are quiet, writes bench/results/<name>-ecs.jsonl and .md, and
# prints the table.
param(
    [int[]]$Entities = @(5000, 10000, 20000),
    [string[]]$Cases = @('light', 'heavy', 'schedule'),
    [int]$Repeats = 3,
    [string]$Name = (Get-Date -Format 'yyyy-MM-dd'),
    [double]$QuietPercent = 15,
    [switch]$SkipBuild,
    [switch]$SummaryOnly
)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$results = Join-Path $PSScriptRoot 'results'
$out = Join-Path $results "$Name-ecs.jsonl"
New-Item -ItemType Directory -Force $results | Out-Null
$target = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $root 'target' }
$exe = Join-Path $target 'release\examples\parallel.exe'

function Wait-Quiet {
    $deadline = (Get-Date).AddMinutes(10)
    while ((Get-Date) -lt $deadline) {
        $busy = (Get-Counter '\Processor(_Total)\% Processor Time' -SampleInterval 2 -MaxSamples 2).CounterSamples | Measure-Object CookedValue -Average
        if ($busy.Average -lt $QuietPercent) { return $busy.Average }
        Write-Host "    waiting: processors $([math]::Round($busy.Average)) % busy"
    }
    return $busy.Average
}

if (-not $SummaryOnly) {
    if (-not $SkipBuild) {
        Push-Location $root
        try { cargo build --release -p bayleaf-ecs --features parallel --example parallel -j 2; if (-not $?) { throw 'the benchmark did not build' } } finally { Pop-Location }
    }
    foreach ($n in $Entities) {
        for ($r = 1; $r -le $Repeats; $r++) {
            foreach ($case in $Cases) {
                foreach ($mode in 'off', 'on') {
                    $busy = Wait-Quiet
                    Write-Host "$case, $n entities, parallel $mode, run $r"
                    & $exe $case $n $mode | Where-Object { $_ -like '{*' } | ForEach-Object { $o = $_ | ConvertFrom-Json; $o | Add-Member busyPercent ([math]::Round([double]$busy)) -Force; $o | ConvertTo-Json -Compress -Depth 5 } | Add-Content $out
                }
            }
        }
    }
}

# Median of the runs' means per case, size and mode; the speed-up is one thread's median over the pool's.
$rows = Get-Content $out | ForEach-Object { $_ | ConvertFrom-Json }
$median = { param($v) $s = @($v | Sort-Object); $s[[int][math]::Floor(($s.Count - 1) / 2)] }
$inv = [cultureinfo]::InvariantCulture
$threads = ($rows | Where-Object { $_.parallel } | Select-Object -First 1).threads
$md = @("# The ECS on several cores, $Name", '', "Median over runs of the mean milliseconds per step; the pool had $threads threads. In brackets, one thread's time over the pool's.", '')
$checks = $rows | Group-Object { "$($_.case)|$($_.entities)" } | Where-Object { @($_.Group | ForEach-Object { ([double]$_.check).ToString('R', $inv) } | Sort-Object -Unique).Count -gt 1 }
$busyRange = $rows | Where-Object { $null -ne $_.busyPercent } | Measure-Object busyPercent -Minimum -Maximum
if ($busyRange.Count) { $md += "Before each case the processors were $($busyRange.Minimum) to $($busyRange.Maximum) % busy (other programs)."; $md += "" }
$md += "Both modes end every case with the same checksum: $(if (-not $checks) { 'yes' } else { 'NO (' + (($checks | ForEach-Object Name) -join ', ') + ')' })."
$md += ''
$sizes = @($rows | ForEach-Object entities | Sort-Object -Unique)
$md += '| Case | Mode | ' + (($sizes | ForEach-Object { "$_ entities" }) -join ' | ') + " | Runs' means, min-max |"
$md += '|---|---|' + (($sizes | ForEach-Object { '---:' }) -join '|') + '|---|'
foreach ($case in @($rows | ForEach-Object case | Select-Object -Unique)) {
    foreach ($parallel in $false, $true) {
        $cells = @(); $spread = @()
        foreach ($n in $sizes) {
            $means = @($rows | Where-Object { $_.case -eq $case -and $_.entities -eq $n -and $_.parallel -eq $parallel } | ForEach-Object { $_.stats.meanMs })
            $single = @($rows | Where-Object { $_.case -eq $case -and $_.entities -eq $n -and -not $_.parallel } | ForEach-Object { $_.stats.meanMs })
            if (-not $means.Count) { $cells += ''; continue }
            $m = & $median $means
            $cells += if ($parallel -and $single.Count) { "{0} ({1})" -f $m.ToString('0.000', $inv), ((& $median $single) / $m).ToString('0.0', $inv) } else { $m.ToString('0.000', $inv) }
            $spread += "{0}-{1}" -f ($means | Measure-Object -Minimum).Minimum.ToString('0.000', $inv), ($means | Measure-Object -Maximum).Maximum.ToString('0.000', $inv)
        }
        $md += "| $case | $(if ($parallel) { 'pool' } else { 'one thread' }) | " + ($cells -join ' | ') + " | $($spread -join '; ') |"
    }
}
$md += ''
[System.IO.File]::WriteAllText((Join-Path $results "$Name-ecs.md"), ($md -join "`n") + "`n")
$md | Write-Host
