param(
    [Parameter(Mandatory)][ValidateSet('Enable', 'Disable')][string]$Action,
    [string]$Exe = (Join-Path $PSScriptRoot 'dist\screenmate.exe')
)
$ErrorActionPreference = 'Stop'
$runPath = 'Software\Microsoft\Windows\CurrentVersion\Run'
$settingsDir = Join-Path $env:LOCALAPPDATA 'Screenmate'
$recordPath = Join-Path $settingsDir 'autostart-preference.json'
$appPath = [System.IO.Path]::GetFullPath($Exe)
$mcpPath = Join-Path (Split-Path -Parent $appPath) 'screenmate-mcp.exe'
if ($Action -eq 'Enable' -and (-not (Test-Path -LiteralPath $appPath -PathType Leaf) -or -not (Test-Path -LiteralPath $mcpPath -PathType Leaf))) {
    throw 'Рядом должны находиться screenmate.exe и screenmate-mcp.exe.'
}
$appCommand = '"' + $appPath + '" --autostart'
$mcpCommand = '"' + $mcpPath + '" --daemon'
$legacyCommand = '"' + $appPath + '"'
if (-not ('ScreenmateAtomicMove' -as [type])) {
    Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class ScreenmateAtomicMove {
    [DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true)]
    public static extern bool MoveFileEx(string source, string destination, uint flags);
}
'@
}
$record = $null
if (Test-Path -LiteralPath $recordPath -PathType Leaf) {
    $record = Get-Content -Raw -LiteralPath $recordPath | ConvertFrom-Json
    if ($record.version -ne 1 -or $record.app -isnot [string] -or $record.mcp -isnot [string] -or $record.enabled -isnot [bool]) { throw 'Некорректные настройки автозапуска.' }
}
function Read-RunValue([string]$Name) {
    $key = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey($runPath, $false)
    if ($null -eq $key) { return $null }
    try {
        if ($key.GetValueNames() -notcontains $Name) { return $null }
        if ($key.GetValueKind($Name) -ne [Microsoft.Win32.RegistryValueKind]::String) { throw 'Обнаружена чужая запись автозапуска; она не изменена.' }
        return [string]$key.GetValue($Name, $null, [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
    } finally { $key.Dispose() }
}
function Write-RunValue([string]$Name, $Value) {
    if ($null -eq $Value) { $key = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey($runPath, $true) }
    else { $key = [Microsoft.Win32.Registry]::CurrentUser.CreateSubKey($runPath) }
    if ($null -eq $key) { if ($null -eq $Value) { return }; throw 'Не удалось открыть раздел автозапуска.' }
    try {
        if ($null -eq $Value) { $key.DeleteValue($Name, $false) }
        else { $key.SetValue($Name, $Value, [Microsoft.Win32.RegistryValueKind]::String) }
    } finally { $key.Dispose() }
}
$names = @('Screenmate', 'ScreenmateMcp')
$original = @((Read-RunValue $names[0]), (Read-RunValue $names[1]))
for ($i=0; $i -lt 2; $i++) {
    if ($null -eq $original[$i]) { continue }
    $expected = if ($i -eq 0) { $appCommand } else { $mcpCommand }
    $old = if ($null -eq $record) { $null } elseif ($i -eq 0) { $record.app } else { $record.mcp }
    $isLegacy = $i -eq 0 -and $original[$i] -ieq $legacyCommand
    if ($original[$i] -ine $expected -and -not $isLegacy -and ($null -eq $old -or $original[$i] -ine $old)) {
        throw 'Обнаружена чужая или изменённая запись автозапуска; она не изменена.'
    }
}
$enabled = $Action -eq 'Enable'
$desired = if ($enabled) { @($appCommand, $mcpCommand) } else { @($null, $null) }
$changed = [System.Collections.Generic.List[int]]::new()
$temp = $null
try {
    for ($i=0; $i -lt 2; $i++) {
        if ($original[$i] -ceq $desired[$i]) { continue }
        if ((Read-RunValue $names[$i]) -cne $original[$i]) { throw 'Запись автозапуска изменилась; повторите действие.' }
        Write-RunValue $names[$i] $desired[$i]
        $changed.Add($i)
    }
    New-Item -ItemType Directory -Force -Path $settingsDir | Out-Null
    $temp = Join-Path $settingsDir ('.autostart-' + [guid]::NewGuid().ToString('N') + '.tmp')
    $json = [ordered]@{version=1;enabled=$enabled;app=$appCommand;mcp=$mcpCommand} | ConvertTo-Json -Compress
    [System.IO.File]::WriteAllText($temp, $json, [System.Text.UTF8Encoding]::new($false))
    if (-not [ScreenmateAtomicMove]::MoveFileEx($temp,$recordPath,0x9)) { throw [System.ComponentModel.Win32Exception]::new([System.Runtime.InteropServices.Marshal]::GetLastWin32Error()) }
    $temp = $null
} catch {
    $originalError = $_
    $rollbackFailed = $false
    for ($j=$changed.Count-1; $j -ge 0; $j--) {
        $i = $changed[$j]
        try {
            if ((Read-RunValue $names[$i]) -cne $desired[$i]) { $rollbackFailed=$true; continue }
            Write-RunValue $names[$i] $original[$i]
        } catch { $rollbackFailed=$true }
    }
    if ($null -ne $temp -and (Test-Path -LiteralPath $temp -PathType Leaf)) { Remove-Item -LiteralPath $temp -ErrorAction SilentlyContinue }
    if ($rollbackFailed) { throw "$originalError; не удалось полностью отменить изменение автозапуска" }
    throw $originalError
}
if ($enabled) { Write-Host 'Записи автозапуска приложения и MCP включены для текущего пользователя.' }
else { Write-Host 'Записи автозапуска приложения и MCP выключены. Уже запущенные процессы продолжают работать.' }
