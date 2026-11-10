# Isolated script integration: registry functions are replaced by an in-memory
# map before execution. No HKCU key or user preferences are read or written.
$ErrorActionPreference='Stop'
$source=Get-Content -Raw -LiteralPath (Join-Path $PSScriptRoot '..\autostart.ps1')
$start=$source.IndexOf('function Read-RunValue')
$end=$source.IndexOf('$names = @', $start)
if ($start -lt 0 -or $end -le $start) {throw 'Cannot locate registry adapter functions'}
$fake=@'
function Read-RunValue([string]$Name) {
    if ($global:ScreenmateFakeRegistry.ContainsKey($Name)) { return $global:ScreenmateFakeRegistry[$Name] }
    return $null
}
function Write-RunValue([string]$Name, $Value) {
    if ($global:ScreenmateFailName -eq $Name) { $global:ScreenmateFailName=$null; throw 'simulated access denied' }
    if ($null -eq $Value) { $global:ScreenmateFakeRegistry.Remove($Name) | Out-Null }
    else { $global:ScreenmateFakeRegistry[$Name]=$Value }
}
'@
$fakeSource=$source.Substring(0,$start)+$fake+"`n"+$source.Substring($end)
$root=Join-Path $env:TEMP ('screenmate-autostart-test-'+[guid]::NewGuid().ToString('N'))
$tempRoot=[System.IO.Path]::GetFullPath($env:TEMP).TrimEnd('\')+'\'
$absolute=[System.IO.Path]::GetFullPath($root)
if (-not $absolute.StartsWith($tempRoot,[System.StringComparison]::OrdinalIgnoreCase)) {throw 'Test path is outside the temporary directory'}
$oldLocal=$env:LOCALAPPDATA
try {
    New-Item -ItemType Directory -Path $root | Out-Null
    $exe=Join-Path $root 'screenmate.exe';$mcp=Join-Path $root 'screenmate-mcp.exe';$script=Join-Path $root 'fake.ps1'
    New-Item -ItemType File -Path $exe,$mcp | Out-Null
    [System.IO.File]::WriteAllText($script,$fakeSource,[System.Text.UTF8Encoding]::new($false))
    $env:LOCALAPPDATA=$root;$global:ScreenmateFakeRegistry=@{};$global:ScreenmateFailName=$null
    & $script -Action Enable -Exe $exe
    if ($global:ScreenmateFakeRegistry['Screenmate'] -cne ('"'+$exe+'" --autostart')) {throw 'Incorrect GUI command'}
    if ($global:ScreenmateFakeRegistry['ScreenmateMcp'] -cne ('"'+$mcp+'" --daemon')) {throw 'Incorrect MCP command'}
    $record=Join-Path $root 'Screenmate\autostart-preference.json'
    $bytes=[System.IO.File]::ReadAllBytes($record)
    if ($bytes.Length -ge 3 -and $bytes[0] -eq 0xef -and $bytes[1] -eq 0xbb -and $bytes[2] -eq 0xbf) {throw 'JSON must not contain a UTF-8 BOM'}
    $decoded=Get-Content -Raw -LiteralPath $record | ConvertFrom-Json
    if (-not $decoded.enabled -or $decoded.app -cne $global:ScreenmateFakeRegistry['Screenmate']) {throw 'Unexpected marker'}
    & $script -Action Disable -Exe $exe
    if ($global:ScreenmateFakeRegistry.Count -ne 0) {throw 'Disable must remove both owned values'}
    if ((Get-Content -Raw -LiteralPath $record | ConvertFrom-Json).enabled) {throw 'Disable marker must persist the opt-out'}
    $global:ScreenmateFailName='ScreenmateMcp'
    $failed=$false;try {& $script -Action Enable -Exe $exe} catch {$failed=$true}
    if (-not $failed -or $global:ScreenmateFakeRegistry.Count -ne 0) {throw 'Partial failure did not roll back'}
    $global:ScreenmateFakeRegistry['Screenmate']='foreign.exe'
    $failed=$false;try {& $script -Action Enable -Exe $exe} catch {$failed=$true}
    if (-not $failed -or $global:ScreenmateFakeRegistry['Screenmate'] -ne 'foreign.exe') {throw 'Foreign registry value was modified'}
    $global:ScreenmateFakeRegistry=@{}
    $bad=Join-Path $root 'bad-local';New-Item -ItemType Directory -Path $bad | Out-Null
    New-Item -ItemType File -Path (Join-Path $bad 'Screenmate') | Out-Null
    $env:LOCALAPPDATA=$bad
    $failed=$false;try {& $script -Action Enable -Exe $exe} catch {$failed=$true}
    if (-not $failed -or $global:ScreenmateFakeRegistry.Count -ne 0) {throw 'Marker write failure did not roll back'}
    Write-Host 'Autostart fake-registry tests passed.'
} finally {
    $env:LOCALAPPDATA=$oldLocal
    Remove-Variable -Scope Global -Name ScreenmateFakeRegistry,ScreenmateFailName -ErrorAction SilentlyContinue
    if (Test-Path -LiteralPath $absolute) {Remove-Item -LiteralPath $absolute -Recurse -Force}
}
