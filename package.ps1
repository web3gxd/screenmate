param([string]$Output = (Join-Path $PSScriptRoot 'Screenmate.zip'))
$ErrorActionPreference = 'Stop'
$root = [IO.Path]::GetFullPath($PSScriptRoot).TrimEnd('\') + '\'
$outputPath = [IO.Path]::GetFullPath($Output)

$files = @(
    '.gitignore', 'README.md', 'AGENTS.screenmate.example.md',
    'build.ps1', 'autostart.ps1', 'package.ps1',
    'app\Cargo.toml', 'app\Cargo.lock', 'app\config.yaml', 'app\build.rs',
    'mcp_server\Cargo.toml', 'mcp_server\Cargo.lock'
)
$folders = @('.github', 'app\src', 'app\assets', 'mcp_server\src', 'mcp_server\tests', 'shared', 'tests', 'design')
$entries = [Collections.Generic.List[object]]::new()
foreach ($relative in $files) {
    $path = [IO.Path]::GetFullPath((Join-Path $root $relative))
    if (-not $path.StartsWith($root, [StringComparison]::OrdinalIgnoreCase) -or -not (Test-Path -LiteralPath $path -PathType Leaf)) { throw "Missing project file: $relative" }
    $entries.Add(@{ Path = $path; Name = $relative.Replace('\', '/') })
}
foreach ($folder in $folders) {
    $path = [IO.Path]::GetFullPath((Join-Path $root $folder))
    if (-not $path.StartsWith($root, [StringComparison]::OrdinalIgnoreCase) -or -not (Test-Path -LiteralPath $path -PathType Container)) { throw "Missing project directory: $folder" }
    Get-ChildItem -LiteralPath $path -File -Recurse -Force | ForEach-Object {
        $full = [IO.Path]::GetFullPath($_.FullName)
        if (-not $full.StartsWith($root, [StringComparison]::OrdinalIgnoreCase) -or ($_.Attributes -band [IO.FileAttributes]::ReparsePoint)) { throw "Unexpected project path: $full" }
        $entries.Add(@{ Path = $full; Name = $full.Substring($root.Length).Replace('\', '/') })
    }
}
foreach ($binary in @(
    @{ Name = 'dist\screenmate.exe'; Candidates = @('app\target\release\screenmate.exe', 'dist\screenmate-update.exe', 'dist\screenmate.exe') },
    @{ Name = 'dist\screenmate-mcp.exe'; Candidates = @('mcp_server\target\release\screenmate-mcp.exe', 'dist\screenmate-mcp.exe') }
)) {
    $compiled = $null
    foreach ($candidate in $binary.Candidates) {
        $path = Join-Path $root $candidate
        if (Test-Path -LiteralPath $path -PathType Leaf) { $compiled = $path; break }
    }
    if ($null -eq $compiled) { throw "Build the project before packaging: $($binary.Name)" }
    $entries.Add(@{ Path = $compiled; Name = $binary.Name.Replace('\', '/') })
}

Add-Type -AssemblyName System.IO.Compression.FileSystem
if (Test-Path -LiteralPath $outputPath) { throw "Archive already exists: $outputPath. Choose a new -Output path." }
$zip = [IO.Compression.ZipFile]::Open($outputPath, [IO.Compression.ZipArchiveMode]::Create)
try {
    foreach ($entry in $entries) {
        [void][IO.Compression.ZipFileExtensions]::CreateEntryFromFile($zip, $entry.Path, "screenmate/$($entry.Name)", [IO.Compression.CompressionLevel]::Optimal)
    }
} finally { $zip.Dispose() }
Write-Host "Created $outputPath"
