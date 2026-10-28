<#
.SYNOPSIS
    Measures a program's memory use (PLAN.md, 15A).

.DESCRIPTION
    CLI programs: runs the program to completion and reports its peak working set.
    GUI programs: pass -IdleSeconds; the script waits that long, reports current and
    peak working set, then closes the program.

.EXAMPLE
    .\scripts\measure-mem.ps1 -Exe .\target\release\pcctl.exe -ArgumentList list
.EXAMPLE
    .\scripts\measure-mem.ps1 -Exe .\target\release\pc-control.exe -IdleSeconds 10 -BudgetMB 25
#>
param(
    [Parameter(Mandatory)][string]$Exe,
    [string[]]$ArgumentList = @(),
    [int]$IdleSeconds = 0,
    [double]$BudgetMB = 0
)

$ErrorActionPreference = 'Stop'

Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class MemInfo {
    [StructLayout(LayoutKind.Sequential)]
    public struct Counters {
        public uint cb; public uint PageFaultCount;
        public UIntPtr PeakWorkingSetSize; public UIntPtr WorkingSetSize;
        public UIntPtr QuotaPeakPagedPoolUsage; public UIntPtr QuotaPagedPoolUsage;
        public UIntPtr QuotaPeakNonPagedPoolUsage; public UIntPtr QuotaNonPagedPoolUsage;
        public UIntPtr PagefileUsage; public UIntPtr PeakPagefileUsage;
    }
    [DllImport("psapi.dll", SetLastError = true)]
    static extern bool GetProcessMemoryInfo(IntPtr process, out Counters counters, uint size);
    public static Counters Read(IntPtr process) {
        Counters c;
        if (!GetProcessMemoryInfo(process, out c, (uint)Marshal.SizeOf(typeof(Counters))))
            throw new System.ComponentModel.Win32Exception(Marshal.GetLastWin32Error());
        return c;
    }
}
'@

$startArgs = @{ FilePath = (Resolve-Path $Exe).Path; PassThru = $true; NoNewWindow = $true }
if ($ArgumentList.Count -gt 0) { $startArgs.ArgumentList = $ArgumentList }
$process = Start-Process @startArgs
# Take the handle right away so the counters stay readable after the process exits.
$handle = $process.Handle

if ($IdleSeconds -gt 0) {
    Start-Sleep -Seconds $IdleSeconds
    $counters = [MemInfo]::Read($handle)
    $process.CloseMainWindow() | Out-Null
    if (-not $process.WaitForExit(3000)) { $process.Kill() }
} else {
    $process.WaitForExit()
    $counters = [MemInfo]::Read($handle)
}

$mb = { param($v) [math]::Round($v.ToUInt64() / 1MB, 2) }
$peak = & $mb $counters.PeakWorkingSetSize
"{0}: peak working set {1} MB, peak private bytes {2} MB" -f (Split-Path $Exe -Leaf), $peak, (& $mb $counters.PeakPagefileUsage)
if ($IdleSeconds -gt 0) {
    "  after {0}s idle: working set {1} MB, private bytes {2} MB" -f $IdleSeconds, (& $mb $counters.WorkingSetSize), (& $mb $counters.PagefileUsage)
}

if ($BudgetMB -gt 0 -and $peak -gt $BudgetMB) {
    Write-Error ("over budget: {0} MB > {1} MB" -f $peak, $BudgetMB)
}
