# The same per-object work in plain C++, C# and Python, with no engine (bench/README.md): builds what
# this computer can build, runs each program -Repeats times per size, writes
# bench/results/<name>-languages.jsonl and .md, and prints the table.
param(
    [int[]]$Objects = @(5000, 10000),
    [int]$Repeats = 3,
    [string]$Name = (Get-Date -Format 'yyyy-MM-dd'),
    [string]$Python = 'python',
    # Which programs run: any of cpp, cs, python (the table always shows every run in the file).
    [string[]]$Languages = @('cpp', 'cs', 'python')
)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$results = Join-Path $PSScriptRoot 'results'
$out = Join-Path $results "$Name-languages.jsonl"
New-Item -ItemType Directory -Force $results | Out-Null

# C++ with MSVC (as bench/run.ps1 builds it).
$cppDir = Join-Path $root 'target\bench-cpp'
$cpp = Join-Path $cppDir 'bench.exe'
$vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
$vs = if (Test-Path $vswhere) { & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath }
if ($vs) {
    New-Item -ItemType Directory -Force $cppDir | Out-Null
    cmd /c "call `"$vs\VC\Auxiliary\Build\vcvars64.bat`" >nul && cd /d `"$cppDir`" && cl /nologo /O2 /EHsc /std:c++17 /Fe:bench.exe `"$(Join-Path $PSScriptRoot 'cpp\bench.cpp')`" >nul"
}

# C# with the compiler that comes with .NET Framework; the same program also runs on .NET (9 or
# later) through a runtimeconfig, when that runtime is installed.
$csDir = Join-Path $root 'target\bench-cs'
$csc = "$env:WINDIR\Microsoft.NET\Framework64\v4.0.30319\csc.exe"
$programs = @()
if ((Test-Path $cpp) -and $Languages -contains 'cpp') { $programs += , @('C++ (MSVC /O2)', $cpp, @()) }
if ((Test-Path $csc) -and $Languages -contains 'cs') {
    New-Item -ItemType Directory -Force $csDir | Out-Null
    & $csc /nologo /optimize+ /platform:x64 /nowarn:0649 "/out:$csDir\bench-cs.exe" (Join-Path $PSScriptRoot 'csharp\Bench.cs')
    if (-not $?) { throw 'the C# program did not build' }
    $programs += , @('C# (.NET Framework 4.8)', "$csDir\bench-cs.exe", @())
    $net = @(dotnet --list-runtimes 2>$null | Where-Object { $_ -match '^Microsoft\.NETCore\.App (\d+)\.' -and [int]$Matches[1] -ge 9 })
    if ($net.Count) {
        $version = ($net[-1] -split ' ')[1]
        Copy-Item "$csDir\bench-cs.exe" "$csDir\bench-cs-net.dll" -Force
        "{`"runtimeOptions`":{`"tfm`":`"net9.0`",`"framework`":{`"name`":`"Microsoft.NETCore.App`",`"version`":`"$version`"}}}" | Set-Content -Encoding ascii "$csDir\bench-cs-net.runtimeconfig.json"
        $programs += , @("C# (.NET $version)", 'dotnet', @("$csDir\bench-cs-net.dll"))
    }
}
# Python: a real interpreter, not the Microsoft Store alias.
$py = Get-Command $Python -ErrorAction SilentlyContinue
if ($Languages -notcontains 'python') { }
elseif ($py -and $py.Source -notlike '*WindowsApps*') { $programs += , @("Python ($(& $Python -c 'import platform; print(platform.python_version())'))", $Python, @((Join-Path $PSScriptRoot 'python\bench.py'))) }
else { Write-Host "Python not found ($Python); skipped." }

foreach ($n in $Objects) {
    for ($r = 1; $r -le $Repeats; $r++) {
        foreach ($p in $programs) {
            Write-Host "$($p[0]), $n objects, run $r"
            & $p[1] @($p[2] + @("$n")) | Where-Object { $_ -like '{*' } | ForEach-Object { $o = $_ | ConvertFrom-Json; $o | Add-Member language $p[0] -Force; $o | ConvertTo-Json -Compress -Depth 5 } | Add-Content $out
        }
    }
}

# Median of the runs' means per language, layout and part; times C++ with objects owning components.
$rows = Get-Content $out | ForEach-Object { $_ | ConvertFrom-Json }
$median = { param($v) $s = @($v | Sort-Object); $s[[int][math]::Floor(($s.Count - 1) / 2)] }
$inv = [cultureinfo]::InvariantCulture
$md = @("# Languages $Name", '', 'Median over runs of the mean milliseconds per step; in brackets, times C++ with objects owning their components.', '')
foreach ($part in 'scripts', 'step') {
    $md += if ($part -eq 'scripts') { '## Script step' } else { '## Script step and world matrices' }
    $md += ''
    $md += '| Version | ' + (($Objects | ForEach-Object { "$_ objects" }) -join ' | ') + " | Runs' means, min-max |"
    $md += '|---|' + (($Objects | ForEach-Object { '---:' }) -join '|') + '|---|'
    foreach ($g in ($rows | Where-Object { $_.part -eq $part } | Group-Object { "$($_.language)|$($_.mode)" })) {
        $first = $g.Group[0]
        $layout = if ($first.mode -like '*packed') { 'packed arrays' } elseif ($first.mode -eq 'python') { 'objects own components' } else { 'objects own components' }
        $cells = @(); $spread = @()
        foreach ($n in $Objects) {
            $means = @($g.Group | Where-Object { $_.objects -eq $n } | ForEach-Object { $_.stats.meanMs })
            $base = @($rows | Where-Object { $_.mode -eq 'cpp-virtual' -and $_.part -eq $part -and $_.objects -eq $n } | ForEach-Object { $_.stats.meanMs })
            if (-not $means.Count) { $cells += ''; continue }
            $m = & $median $means
            $x = if ($base.Count) { $m / (& $median $base) } else { 0 }
            $cells += "{0} ({1})" -f $m.ToString('0.000', $inv), $x.ToString($(if ($x -ge 10) { '0' } else { '0.0' }), $inv)
            $spread += "{0}-{1}" -f ($means | Measure-Object -Minimum).Minimum.ToString('0.000', $inv), ($means | Measure-Object -Maximum).Maximum.ToString('0.000', $inv)
        }
        $md += "| $($first.language), $layout | " + ($cells -join ' | ') + " | $($spread -join '; ') |"
    }
    $md += ''
}
[System.IO.File]::WriteAllText((Join-Path $results "$Name-languages.md"), ($md -join "`n") + "`n")
$md | Write-Host
