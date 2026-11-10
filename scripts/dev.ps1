# Start the whole stack on Windows in mock mode (no GPU needed):
#   API  http://127.0.0.1:8080   Web  http://localhost:5173   Worker with the mock engine
# Usage:  powershell -ExecutionPolicy Bypass -File scripts\dev.ps1
# Each service opens in its own window; close a window to stop that service.
# Dev data lives in apps\api\data (ignored by git). Login: admin / admin-dev-password

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$api = Join-Path $root 'apps\api'
$web = Join-Path $root 'apps\web'
$worker = Join-Path $root 'workers\music-worker'
$base = 'http://127.0.0.1:8080'
$adminUser = 'admin'
$adminPassword = 'admin-dev-password'

function Start-Window($title, $dir, $command) {
    Start-Process powershell -WorkingDirectory $dir -ArgumentList '-NoExit', '-Command', "`$Host.UI.RawUI.WindowTitle = '$title'; $command"
}

# --- API ---
$env:BOOTSTRAP_ADMIN_USERNAME = $adminUser
$env:BOOTSTRAP_ADMIN_PASSWORD = $adminPassword
Start-Window 'lush-api' $api 'cargo run'
Write-Host 'Waiting for the API (first build can take a few minutes)...'
$ready = $false
foreach ($i in 1..600) {
    try { Invoke-RestMethod "$base/api/health" | Out-Null; $ready = $true; break } catch { Start-Sleep -Seconds 1 }
}
if (-not $ready) { throw 'API did not start' }

# --- Worker: create the dev node once and keep its token in workers\music-worker\.env ---
$envFile = Join-Path $worker '.env'
if (-not (Test-Path $envFile)) {
    $session = New-Object Microsoft.PowerShell.Commands.WebRequestSession
    $login = @{ username = $adminUser; password = $adminPassword } | ConvertTo-Json
    Invoke-RestMethod "$base/api/v1/auth/login" -Method Post -ContentType 'application/json' -Body $login -WebSession $session | Out-Null
    $node = Invoke-RestMethod "$base/api/v1/nodes" -Method Post -ContentType 'application/json' -Body '{"name":"dev-node-01"}' -WebSession $session
    @("CONTROL_PLANE_URL=$base", "NODE_TOKEN=$($node.token)", 'WORKER_ENGINES=mock', 'MOCK_STEP_DELAY=0.5') |
        Set-Content -Path $envFile -Encoding ascii
    Write-Host 'Created dev-node-01 and wrote workers\music-worker\.env'
}
if (-not (Test-Path (Join-Path $worker '.venv'))) {
    Push-Location $worker
    py -m venv .venv
    & .\.venv\Scripts\python -m pip install --quiet -e '.[dev]'
    Pop-Location
}
Start-Window 'lush-worker' $worker '.\.venv\Scripts\python -m lush_worker'

# --- Web ---
if (-not (Test-Path (Join-Path $web 'node_modules'))) {
    Push-Location $web; npm install; Pop-Location
}
Start-Window 'lush-web' $web 'npm run dev'

Write-Host ''
Write-Host "Open http://localhost:5173  (login: $adminUser / $adminPassword)"
