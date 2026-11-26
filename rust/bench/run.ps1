# Runs the Bayleaf and C++ benchmarks one case at a time and writes their JSON lines to
# bench/results/<name>.jsonl, then prints a summary table. See bench/README.md.
#
# Each case runs in its own process, one after another, and is stopped when its private memory
# passes -MemoryLimitGB, so a case that does not fit fails alone instead of starving the computer.
param(
    [int[]]$Objects = @(500, 1000, 5000, 10000, 20000),
    # Sizes at which the whole play step also runs with scripts (play.start loads them slowly).
    [int[]]$StepObjects = @(500, 1000),
    # Largest size for the cases on the component-fields path (their loading grows with scripts squared)...
    [int]$HeavyMaxObjects = 5000,
    # ...and the largest at which they repeat.
    [int]$HeavyRepeatObjects = 1000,
    [int]$Repeats = 3,
    [double]$MemoryLimitGB = 12,
    # Each case starts once the processors are less busy than this (percent, all of them together).
    [double]$QuietPercent = 15,
    [string]$Name = (Get-Date -Format 'yyyy-MM-dd-HHmm'),
    [switch]$SkipBuild,
    [switch]$SummaryOnly
)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$results = Join-Path $PSScriptRoot 'results'
$out = Join-Path $results "$Name.jsonl"
$bench = Join-Path $root 'target\release\bayleaf-bench.exe'
$cppDir = Join-Path $root 'target\bench-cpp'
$cpp = Join-Path $cppDir 'bench.exe'

function Build-All {
    # The scripts read bench/scripts/.cargo/config.toml (a small stack), so build from that folder.
    Push-Location (Join-Path $PSScriptRoot 'scripts')
    try { cargo build --release --target wasm32-unknown-unknown -j 2; if (-not $?) { throw 'the scripts did not build' } } finally { Pop-Location }
    cargo build --release -p bayleaf-bench -j 2 --manifest-path (Join-Path $root 'Cargo.toml')
    if (-not $?) { throw 'the benchmark did not build' }
    $vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
    $vs = & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
    if (-not $vs) { throw 'no MSVC (Visual Studio C++ build tools) found for the C++ program' }
    New-Item -ItemType Directory -Force $cppDir | Out-Null
    $src = Join-Path $PSScriptRoot 'cpp\bench.cpp'
    cmd /c "call `"$vs\VC\Auxiliary\Build\vcvars64.bat`" >nul && cd /d `"$cppDir`" && cl /nologo /O2 /EHsc /std:c++17 /Fe:bench.exe `"$src`""
    if (-not $?) { throw 'the C++ program did not build' }
}

# Runs one program to the end, or stops it past the memory limit; returns its JSON lines.
function Invoke-Case([string]$program, [string[]]$arguments, [hashtable]$what) {
    $stdout = [System.IO.Path]::GetTempFileName()
    $stderr = [System.IO.Path]::GetTempFileName()
    $p = Start-Process -FilePath $program -ArgumentList $arguments -NoNewWindow -PassThru -RedirectStandardOutput $stdout -RedirectStandardError $stderr
    # Read the handle now, or PowerShell forgets the exit code of a process it did not wait for.
    $null = $p.Handle
    [long]$peak = 0
    $killed = $false
    while (-not $p.HasExited) {
        Start-Sleep -Milliseconds 200
        try { $p.Refresh(); $peak = [math]::Max([long]$peak, [long]$p.PrivateMemorySize64) } catch {}
        if ($peak -gt $MemoryLimitGB * 1GB) { $p.Kill(); $killed = $true; break }
    }
    $p.WaitForExit()
    $lines = @(Get-Content $stdout | Where-Object { $_ -like '{*' })
    $err = [string](Get-Content $stderr -Raw)
    Remove-Item $stdout, $stderr
    $peakGB = [math]::Round($peak / 1GB, 2)
    if ($killed -or $p.ExitCode -ne 0 -or $lines.Count -eq 0) {
        $why = if ($killed) { "stopped: private memory passed $MemoryLimitGB GB" } else { "exit $($p.ExitCode): $("$err".Trim())" }
        $fail = $what.Clone(); $fail.error = $why; $fail.peakGB = $peakGB
        return @(($fail | ConvertTo-Json -Compress))
    }
    return @($lines | ForEach-Object { $o = $_ | ConvertFrom-Json; $o | Add-Member peakGB $peakGB -Force; $o | ConvertTo-Json -Compress -Depth 5 })
}

# Waits (up to 10 minutes) until the processors are mostly idle, so another program's work (a build)
# does not land in a measurement.
function Wait-Quiet {
    $deadline = (Get-Date).AddMinutes(10)
    while ((Get-Date) -lt $deadline) {
        $busy = (Get-Counter '\Processor(_Total)\% Processor Time' -SampleInterval 2 -MaxSamples 2).CounterSamples | Measure-Object CookedValue -Average
        if ($busy.Average -lt $QuietPercent) { return }
        Write-Host "    waiting: processors $([math]::Round($busy.Average)) % busy"
    }
}

function Invoke-Bayleaf([string]$mode, [string]$scripts, [string]$part, [int]$n, [string[]]$extra = @()) {
    $what = @{ mode = $mode; scripts = $scripts; part = $part; objects = $n }
    Wait-Quiet
    Write-Host "  $mode $scripts $part $extra"
    Invoke-Case $bench (@('--mode', $mode, '--scripts', $scripts, '--part', $part, '--objects', "$n") + $extra) $what | Add-Content $out
}

function Invoke-All {
    New-Item -ItemType Directory -Force $results | Out-Null
    foreach ($n in $Objects) {
        for ($r = 1; $r -le $Repeats; $r++) {
            Write-Host "objects $n, run $r of $Repeats"
            Wait-Quiet
            Invoke-Case $cpp @("$n") @{ mode = 'cpp'; objects = $n } | Add-Content $out
            # Visual scripts, native mode and the `fields` control take the host's component-fields
            # path, whose loading grows with scripts x scripts (README): up to -HeavyMaxObjects, and
            # past -HeavyRepeatObjects only in the first run.
            $heavy = $n -le $HeavyMaxObjects -and ($r -eq 1 -or $n -le $HeavyRepeatObjects)
            # The script step, and what it is made of: the world view alone (no scripts), running
            # scripts that do nothing, and the same with the component-fields path on.
            Invoke-Bayleaf 'interpreted' 'none' 'scripts' $n
            foreach ($mode in 'interpreted', 'compiled') { Invoke-Bayleaf $mode 'empty' 'scripts' $n }
            if ($heavy) {
                foreach ($mode in 'interpreted', 'compiled') { Invoke-Bayleaf $mode 'fields' 'scripts' $n }
                Invoke-Bayleaf 'native' 'empty' 'scripts' $n
            }
            # The benchmark's work: spin, bob and follow on every object.
            foreach ($mode in 'interpreted', 'compiled') { Invoke-Bayleaf $mode 'rust' 'scripts' $n }
            if ($heavy) {
                foreach ($mode in 'interpreted', 'compiled') { Invoke-Bayleaf $mode 'graph' 'scripts' $n }
                Invoke-Bayleaf 'native' 'rust' 'scripts' $n
            }
            # The whole play step. play.start loads scripts against the whole scene for every
            # instance, so with scripts only at -StepObjects sizes.
            Invoke-Bayleaf 'interpreted' 'none' 'step' $n
            if ($StepObjects -contains $n) {
                foreach ($c in @('interpreted', 'rust'), @('interpreted', 'graph'), @('compiled', 'rust'), @('compiled', 'graph'), @('native', 'rust')) {
                    Invoke-Bayleaf $c[0] $c[1] 'step' $n
                }
                # The script step loaded as play mode loads it, to show that the quick load times the same steps.
                Invoke-Bayleaf 'interpreted' 'rust' 'scripts' $n @('--full-load')
            }
        }
    }
}

# Median over the runs of each case's mean, the spread of those means, and the ratio to C++.
function Show-Summary {
    $rows = Get-Content $out | ForEach-Object { $_ | ConvertFrom-Json }
    $ok = $rows | Where-Object { -not $_.error }
    $groups = $ok | Group-Object { "$($_.mode)|$($_.scripts)|$($_.part)|$($_.objects)|$($_.fullLoad)" }
    $median = { param($v) $s = @($v | Sort-Object); $s[[int][math]::Floor(($s.Count - 1) / 2)] }
    $table = foreach ($g in $groups) {
        $means = $g.Group | ForEach-Object { $_.stats.meanMs }
        $first = $g.Group[0]
        $base = $ok | Where-Object { $_.mode -eq 'cpp-virtual' -and $_.part -eq $first.part -and $_.objects -eq $first.objects } | ForEach-Object { $_.stats.meanMs }
        $m = & $median $means
        $b = if ($base) { & $median $base } else { $null }
        [pscustomobject]@{
            objects = $first.objects; part = $first.part; mode = $first.mode; scripts = $first.scripts; load = if ($first.fullLoad) { "play" } else { "" }
            ms = [math]::Round($m, 3)
            spread = "{0:N3}-{1:N3}" -f ($means | Measure-Object -Minimum).Minimum, ($means | Measure-Object -Maximum).Maximum
            stdevMs = [math]::Round((& $median ($g.Group | ForEach-Object { $_.stats.stdevMs })), 3)
            xCpp = if ($b) { [math]::Round($m / $b, 1) } else { '' }
            peakGB = (& $median ($g.Group | ForEach-Object { $_.peakGB }))
            runs = $g.Count
            loadS = if ($first.loadSeconds) { [math]::Round((& $median ($g.Group | ForEach-Object { $_.loadSeconds })), 1) } else { '' }
            min = ($means | Measure-Object -Minimum).Minimum
            max = ($means | Measure-Object -Maximum).Maximum
        }
    }
    $table | Sort-Object objects, part, ms | Format-Table -AutoSize objects, part, mode, scripts, load, ms, spread, stdevMs, xCpp, peakGB, runs, loadS | Out-String -Width 200 | Write-Host
    $rows | Where-Object { $_.error } | ForEach-Object { Write-Host "failed: $($_.mode) $($_.scripts) $($_.part) $($_.objects): $($_.error)" }
    Write-Markdown $table $rows
}

# The summary as Markdown, beside the JSON lines: a table per part (objects across), then every case.
function Write-Markdown($table, $rows) {
    $inv = [cultureinfo]::InvariantCulture
    $num = { param($v) if ($v -ge 100) { $v.ToString('0', $inv) } elseif ($v -ge 1) { $v.ToString('0.0', $inv) } else { $v.ToString('0.000', $inv) } }
    $ratio = { param($v) if ($v -ge 10) { $v.ToString('0', $inv) } else { $v.ToString('0.0', $inv) } }
    $labels = [ordered]@{
        'cpp-virtual|cpp' = 'C++, objects own their components (virtual update)'
        'cpp-packed|cpp' = 'C++, packed arrays'
        'interpreted|rust' = 'Bayleaf interpreted, Rust scripts'
        'interpreted|graph' = 'Bayleaf interpreted, visual scripts'
        'compiled|rust' = 'Bayleaf compiled, Rust scripts'
        'compiled|graph' = 'Bayleaf compiled, visual scripts'
        'native|rust' = 'Bayleaf native, Rust scripts'
        'interpreted|none' = 'Bayleaf, no scripts'
        'interpreted|empty' = 'Bayleaf interpreted, 3 empty scripts'
        'compiled|empty' = 'Bayleaf compiled, 3 empty scripts'
        'native|empty' = 'Bayleaf native, 3 empty scripts'
        'interpreted|fields' = 'Bayleaf interpreted, 3 empty scripts on the fields path'
        'compiled|fields' = 'Bayleaf compiled, 3 empty scripts on the fields path'
    }
    $sizes = @($table | ForEach-Object { [int]$_.objects } | Sort-Object -Unique)
    $md = @("# Results $Name", '', 'Median over runs of the mean milliseconds per step; in brackets, times the C++ program with objects owning their components.', '')
    foreach ($part in 'scripts', 'step') {
        $md += if ($part -eq 'scripts') { '## Script step' } else { '## Whole play step' }
        $md += ''
        $md += '| Version | ' + (($sizes | ForEach-Object { "$_ objects" }) -join ' | ') + ' |'
        $md += '|---|' + (($sizes | ForEach-Object { '---:' }) -join '|') + '|'
        foreach ($key in $labels.Keys) {
            $cells = foreach ($n in $sizes) {
                $row = $table | Where-Object { "$($_.mode)|$($_.scripts)" -eq $key -and $_.part -eq $part -and $_.objects -eq $n -and -not $_.load }
                if ($row) { $x = if ($row.xCpp -ne '') { " ($((& $ratio ([double]$row.xCpp))) x)" } else { '' }; "$(& $num $row.ms)$x" } else { '' }
            }
            if (($cells | Where-Object { $_ }).Count) { $md += "| $($labels[$key]) | " + ($cells -join ' | ') + ' |' }
        }
        $md += ''
    }
    $md += '## Every case'
    $md += ''
    $md += '| Objects | Part | Mode | Scripts | Load | Median ms | Runs'' means, min-max | Stdev within a run | x C++ | Peak GB | Runs | Load s |'
    $md += '|---:|---|---|---|---|---:|---:|---:|---:|---:|---:|---:|'
    foreach ($r in ($table | Sort-Object objects, part, ms)) {
        $md += "| $($r.objects) | $($r.part) | $($r.mode) | $($r.scripts) | $($r.load) | $(& $num $r.ms) | $(& $num $r.min)-$(& $num $r.max) | $(& $num $r.stdevMs) | $($r.xCpp) | $($r.peakGB) | $($r.runs) | $($r.loadS) |"
    }
    $failed = @($rows | Where-Object { $_.error })
    if ($failed.Count) {
        $md += ''
        $md += '## Failed'
        $md += ''
        foreach ($f in $failed) { $md += "- $($f.mode) $($f.scripts) $($f.part), $($f.objects) objects: $($f.error)" }
    }
    [System.IO.File]::WriteAllText((Join-Path $results "$Name.md"), ($md -join "`n") + "`n")
}

if (-not $SummaryOnly) {
    if (-not $SkipBuild) { Build-All }
    Invoke-All
}
Show-Summary
