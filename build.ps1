param([switch]$SkipTests)
$ErrorActionPreference = 'Stop'
$root = $PSScriptRoot
if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    throw 'Cargo не найден. Установите Rust и откройте новый терминал.'
}
foreach ($part in @('mcp_server', 'app')) {
    $manifest = Join-Path $root "$part\Cargo.toml"
    if (-not $SkipTests) {
        & cargo test --manifest-path $manifest
        if ($LASTEXITCODE -ne 0) { throw "Тесты $part не прошли" }
    }
    & cargo build --release --manifest-path $manifest
    if ($LASTEXITCODE -ne 0) { throw "Сборка $part не удалась" }
}
$dist = Join-Path $root 'dist'
New-Item -ItemType Directory -Force -Path $dist | Out-Null
Copy-Item (Join-Path $root 'app\target\release\screenmate.exe') $dist -Force
Copy-Item (Join-Path $root 'mcp_server\target\release\screenmate-mcp.exe') $dist -Force
Write-Host "Готово: $dist"
