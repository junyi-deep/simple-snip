param(
    [string]$Executable = "$PSScriptRoot/../target/release/simple-snip.exe",
    [ValidatePattern('^[a-zA-Z0-9_-]+$')][string]$Label = 'local',
    [ValidateRange(512,16384)][int]$MemoryLimitMiB = 6144
)
$ErrorActionPreference = 'Stop'
$exe = (Resolve-Path -LiteralPath $Executable).Path
$folder = Join-Path $PSScriptRoot "../docs/verification/perf-$Label"
New-Item -ItemType Directory -Force -Path $folder | Out-Null
$folder = (Resolve-Path -LiteralPath $folder).Path
$env:RUST_LOG = 'simple_snip=info,snip_platform=warn'
$stdout = Join-Path $folder 'native.log'
$stderr = Join-Path $folder 'stderr.log'
$cpu = Get-CimInstance Win32_Processor | Select-Object -First 1
$memory = (Get-CimInstance Win32_ComputerSystem).TotalPhysicalMemory
$logical = [Environment]::ProcessorCount
$metadata = [ordered]@{ executable=$exe; sha256=(Get-FileHash -LiteralPath $exe -Algorithm SHA256).Hash; cpu=$cpu.Name; logicalProcessors=$logical; physicalMemoryBytes=$memory; sampleIntervalMs=250; command='--perf-test' }
$metadata | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $folder 'machine.json') -Encoding utf8
# Start only this explicitly selected test executable. Existing user instances
# retain their own windows, preferences and hotkeys.
$probe = Start-Process -FilePath $exe -ArgumentList '--perf-test' -WindowStyle Hidden -PassThru -RedirectStandardOutput $stdout -RedirectStandardError $stderr
$clock = [Diagnostics.Stopwatch]::StartNew()
$samples = [Collections.Generic.List[object]]::new()
$previousCpu = 0.0
$previousTime = 0.0
$failure = $null
try {
    while (!$probe.HasExited) {
        $probe.Refresh()
        $time = $clock.Elapsed.TotalSeconds
        $processor = $probe.TotalProcessorTime.TotalSeconds
        $delta = $time - $previousTime
        $core = if ($delta -gt 0) { ($processor-$previousCpu)/$delta*100 } else { 0 }
        $samples.Add([pscustomobject]@{ elapsedMs=[math]::Round($time*1000); utc=[DateTime]::UtcNow.ToString('o'); workingSetMiB=$probe.WorkingSet64/1MB; privateMiB=$probe.PrivateMemorySize64/1MB; cpuCorePercent=$core; cpuTotalPercent=$core/$logical; handles=$probe.HandleCount; threads=$probe.Threads.Count })
        $previousCpu=$processor
        $previousTime=$time
        if ($probe.PrivateMemorySize64/1MB -gt $MemoryLimitMiB) { throw "Test process exceeded ${MemoryLimitMiB} MiB" }
        if ($time -gt 360) { throw 'Test process exceeded six minute deadline' }
        Start-Sleep -Milliseconds 250
    }
    if ($probe.ExitCode -ne 0) { throw "Test process exited with $($probe.ExitCode)" }
    if (!(Select-String -LiteralPath $stdout -SimpleMatch 'PERF PASS' -Quiet)) { throw 'Native test did not report PERF PASS' }
} catch {
    $failure = $_.Exception.Message
    if (!$probe.HasExited) { Stop-Process -Id $probe.Id }
} finally {
    $samples | Export-Csv -LiteralPath (Join-Path $folder 'process.csv') -NoTypeInformation -Encoding utf8
}
if ($failure) { throw $failure }
Write-Output "Performance capture complete: $folder"
