# The same small runtime in C++, Rust and C#, and as one WebAssembly module (bench/README.md, "The same
# runtime in C++ and Rust"): builds what this computer can build, runs each program -Repeats times
# per size, writes bench/results/<name>-runtime.jsonl and .md, and prints the table.
param(
    [int[]]$Objects = @(5000, 20000, 100000),
    [int]$Repeats = 3,
    [string]$Name = (Get-Date -Format 'yyyy-MM-dd'),
    # Which programs run: any of cpp, rust, wasm, csharp (the table always shows every run in the file).
    [string[]]$Programs = @('cpp', 'rust', 'wasm', 'csharp'),
    # Where the WebAssembly host is built; wasmtime takes several minutes the first time.
    [string]$WasmHostTarget = ''
)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$results = Join-Path $PSScriptRoot 'results'
$out = Join-Path $results "$Name-runtime.jsonl"
New-Item -ItemType Directory -Force $results | Out-Null
$runs = @()

# Runs a build command; it failed when its exit code says so. Windows PowerShell 5.1 would otherwise
# stop the script at the first warning a compiler writes to stderr.
function Invoke-Build([string]$What, [scriptblock]$Command) {
    $ErrorActionPreference = 'Continue'
    & $Command 2>&1 | Out-Host
    if ($LASTEXITCODE -ne 0) { throw "$What did not build" }
}

# C++ with MSVC.
$cppDir = Join-Path $root 'target\bench-runtime-cpp'
$cpp = Join-Path $cppDir 'runtime.exe'
$vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
$vs = if (Test-Path $vswhere) { & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath }
if ($vs -and $Programs -contains 'cpp') {
    New-Item -ItemType Directory -Force $cppDir | Out-Null
    $src = Join-Path $PSScriptRoot 'runtime\cpp'
    cmd /c "call `"$vs\VC\Auxiliary\Build\vcvars64.bat`" >nul 2>nul && cd /d `"$cppDir`" && cl /nologo /O2 /EHsc /std:c++17 /W4 /Fe:runtime.exe `"$src\main.cpp`" `"$src\game.cpp`" >nul"
    if (-not (Test-Path $cpp)) { throw 'the C++ program did not build' }
    $runs += , @('C++ (MSVC /O2)', $cpp, @())
}

# Rust, native and for WebAssembly, from the same sources.
$rustDir = Join-Path $PSScriptRoot 'runtime\rust'
$rustTarget = Join-Path $root 'target\bench-runtime-rust'
if ($Programs -contains 'rust' -or $Programs -contains 'wasm') {
    Push-Location $rustDir
    try {
        $env:CARGO_TARGET_DIR = $rustTarget
        Invoke-Build 'the Rust program' { cargo build --release -j 2 }
        if ($Programs -contains 'wasm') { Invoke-Build 'the module' { cargo build --release --lib --target wasm32-unknown-unknown -j 2 } }
    } finally { Pop-Location }
    if ($Programs -contains 'rust') { $runs += , @("Rust ($((rustc --version) -replace '^rustc (\S+).*', '$1'))", (Join-Path $rustTarget 'release\runtime-rust.exe'), @()) }
}
if ($Programs -contains 'wasm') {
    $hostTarget = if ($WasmHostTarget) { $WasmHostTarget } else { Join-Path $root 'target\bench-runtime-wasm-host' }
    Push-Location (Join-Path $PSScriptRoot 'runtime\wasm-host')
    try {
        $env:CARGO_TARGET_DIR = $hostTarget
        Invoke-Build 'the WebAssembly host' { cargo build --release -j 2 }
    } finally { Pop-Location }
    $runs += , @('WebAssembly, one module', (Join-Path $hostTarget 'release\runtime-wasm-host.exe'), @((Join-Path $rustTarget 'wasm32-unknown-unknown\release\runtime_rust.wasm')))
}
Remove-Item Env:CARGO_TARGET_DIR -ErrorAction SilentlyContinue

# C# on .NET 10 (docs/csharp-redesign.md, stage 0): on the JIT and as NativeAOT, from the same sources.
# NativeAOT links with the Visual Studio tools, which it finds through vswhere.
$dotnet = Get-Command dotnet -ErrorAction SilentlyContinue
if ($dotnet -and $Programs -contains 'csharp') {
    $env:DOTNET_CLI_TELEMETRY_OPTOUT = '1'; $env:DOTNET_NOLOGO = '1'
    $env:PATH += ";${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer"
    $project = Join-Path $PSScriptRoot 'runtime\csharp\runtime-csharp.csproj'
    Invoke-Build 'the C# program' { & $dotnet build $project -c Release }
    Invoke-Build 'the NativeAOT program' { & $dotnet publish $project -c Release -r win-x64 -p:PublishAot=true }
    $sdk = (& $dotnet --version)
    $csDir = Join-Path $root 'target\bench-runtime-csharp'
    $runs += , @("C# 14 (.NET SDK $sdk)", (Join-Path $csDir 'bin\runtime-csharp\release\runtime-csharp.exe'), @())
    $runs += , @("C# 14 (.NET SDK $sdk)", (Join-Path $csDir 'publish\runtime-csharp\release_win-x64\runtime-csharp.exe'), @())
}

foreach ($n in $Objects) {
    for ($r = 1; $r -le $Repeats; $r++) {
        foreach ($p in $runs) {
            Write-Host "$($p[0]), $n objects, run $r"
            & $p[1] @($p[2] + @("$n")) | Where-Object { $_ -like '{*' } | ForEach-Object { $o = $_ | ConvertFrom-Json; $o | Add-Member language $p[0] -Force; $o | ConvertTo-Json -Compress -Depth 5 } | Add-Content $out
        }
    }
}

# Median of the runs' means per program, style and part; times the C++ runtime with typed scripts.
$rows = Get-Content $out | ForEach-Object { $_ | ConvertFrom-Json }
$median = { param($v) $s = @($v | Sort-Object); $s[[int][math]::Floor(($s.Count - 1) / 2)] }
$inv = [cultureinfo]::InvariantCulture
$label = {
    param($mode)
    $what = switch -Wildcard ($mode) {
        '*-typed-copy' { 'typed scripts, transforms copied in and out' }
        '*-typed' { 'typed scripts' }
        '*-virtual' { 'a virtual call per script' }
    }
    $how = switch -Wildcard ($mode) {
        'wasm-compiled-*' { ', compiled' }
        'wasm-interpreted-*' { ', interpreted' }
        'csharp-jit-*' { ', JIT' }
        'csharp-aot-*' { ', NativeAOT' }
        default { '' }
    }
    "$how, $what"
}
$md = @("# The same runtime in C++, Rust and C#, $Name", '', 'Median over runs of the mean milliseconds per step; in brackets, times the C++ runtime with typed scripts.', '')
$checks = @($rows | ForEach-Object { '{0}:{1}' -f $_.objects, ([double]$_.check).ToString('0.000000000e+00', $inv) } | Sort-Object -Unique)
$md += "Every version agrees on the checksum after 100 steps: $(if ($checks.Count -eq @($Objects).Count) { 'yes' } else { 'NO (' + ($checks -join ', ') + ')' })."
$md += ''
foreach ($part in 'scripts', 'step') {
    $md += if ($part -eq 'scripts') { '## Script step' } else { '## Script step, world matrices of what moved, and reading the moved list' }
    $md += ''
    $md += '| Version | ' + (($Objects | ForEach-Object { "$_ objects" }) -join ' | ') + " | Runs' means, min-max |"
    $md += '|---|' + (($Objects | ForEach-Object { '---:' }) -join '|') + '|---|'
    foreach ($g in ($rows | Where-Object { $_.part -eq $part } | Group-Object { "$($_.language)|$($_.mode)" })) {
        $first = $g.Group[0]
        $cells = @(); $spread = @()
        foreach ($n in $Objects) {
            $means = @($g.Group | Where-Object { $_.objects -eq $n } | ForEach-Object { $_.stats.meanMs })
            $base = @($rows | Where-Object { $_.mode -eq 'cpp-runtime-typed' -and $_.part -eq $part -and $_.objects -eq $n } | ForEach-Object { $_.stats.meanMs })
            if (-not $means.Count) { $cells += ''; continue }
            $m = & $median $means
            $x = if ($base.Count) { $m / (& $median $base) } else { 0 }
            $cells += "{0} ({1})" -f $m.ToString('0.000', $inv), $x.ToString($(if ($x -ge 10) { '0' } else { '0.0' }), $inv)
            $spread += "{0}-{1}" -f ($means | Measure-Object -Minimum).Minimum.ToString('0.000', $inv), ($means | Measure-Object -Maximum).Maximum.ToString('0.000', $inv)
        }
        $md += "| $($first.language)$(& $label $first.mode) | " + ($cells -join ' | ') + " | $($spread -join '; ') |"
    }
    $md += ''
}
[System.IO.File]::WriteAllText((Join-Path $results "$Name-runtime.md"), ($md -join "`n") + "`n")
$md | Write-Host
