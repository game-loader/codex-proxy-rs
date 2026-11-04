# Codex Proxy RS 一键配置：升级 Codex CLI 到最新版，并按网关模板写入用户配置
# 由前端构建生成，配置模板来自 src/utils/codex-config.ts
# 脚本常经 irm 以脚本块运行，失败时 throw 而不是 exit，避免关闭用户的 PowerShell 窗口
param(
    [Parameter(Mandatory = $true)][string]$BaseUrl,
    [Parameter(Mandatory = $true)][string]$ApiKey,
    [switch]$WebSocket
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"

$CodexReleasesUrl = "https://releases.openai.com/codex"
$CodexInstallerUrl = "https://chatgpt.com/codex/install.ps1"
# 模板以 UTF-8 Base64 存放，脚本保持纯 ASCII，不受下载时的文本编码推断影响
$ConfigTemplate = [System.Text.Encoding]::UTF8.GetString([System.Convert]::FromBase64String("@@CODEX_CONFIG_BASE64@@"))

function Write-Step([string]$Message) {
    Write-Host "==> $Message"
}

function Write-WarningStep([string]$Message) {
    Write-Host "WARNING: $Message" -ForegroundColor Yellow
}

# 写入 TOML 基本字符串；控制字符无法安全写入单行值，直接拒绝
function ConvertTo-TomlString([string]$Value, [string]$Name) {
    if ($Value -match "[\x00-\x1f\x7f]") {
        throw "$Name contains control characters."
    }
    return '"' + $Value.Replace('\', '\\').Replace('"', '\"') + '"'
}

# 只选可执行程序，避免执行策略拦截 npm 生成的 codex.ps1
function Resolve-Codex {
    $command = Get-Command codex -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
    if ($null -ne $command) {
        return $command.Source
    }
    $standaloneDir = if ([string]::IsNullOrWhiteSpace($env:CODEX_INSTALL_DIR)) {
        Join-Path $env:LOCALAPPDATA "Programs\OpenAI\Codex\bin"
    } else {
        $env:CODEX_INSTALL_DIR
    }
    $standalonePath = Join-Path $standaloneDir "codex.exe"
    if (Test-Path -LiteralPath $standalonePath) {
        return $standalonePath
    }
    return $null
}

function Get-CodexVersion([string]$CodexPath) {
    try {
        $output = & $CodexPath --version 2>$null | Select-Object -First 1
        if ($output) {
            return ("$output".Trim() -split "\s+")[-1]
        }
    } catch {
    }
    return $null
}

function Get-LatestCodexVersion {
    try {
        $release = Invoke-RestMethod -Uri "$CodexReleasesUrl/channels/latest" -TimeoutSec 30
        if ("$($release.tag_name)" -match "^rust-v(.+)$") {
            return $Matches[1]
        }
    } catch {
    }
    return $null
}

# 与官方安装器的判定一致：npm／bun 的全局目录归对应包管理器，其余交给官方独立安装
function Get-CodexManager([string]$CodexPath) {
    if ($CodexPath -match "\\\.bun\\") {
        return "bun"
    }
    if ($CodexPath -match "node_modules" -or $CodexPath -match "\\npm\\") {
        return "npm"
    }
    return "standalone"
}

function Invoke-NativeStep([string]$FilePath, [string[]]$Arguments) {
    Write-Step "Running: $FilePath $($Arguments -join ' ')"
    & $FilePath @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "$FilePath exited with code $LASTEXITCODE."
    }
}

# 官方安装器可能调用 exit，放到子进程中运行，避免结束当前会话
function Invoke-OfficialInstaller {
    $installer = Join-Path ([System.IO.Path]::GetTempPath()) ("codex-install-" + [System.Guid]::NewGuid().ToString("N") + ".ps1")
    $previousNonInteractive = $env:CODEX_NON_INTERACTIVE
    try {
        Invoke-WebRequest -Uri $CodexInstallerUrl -OutFile $installer -UseBasicParsing -TimeoutSec 120
        $env:CODEX_NON_INTERACTIVE = "1"
        $powershell = (Get-Process -Id $PID).Path
        Invoke-NativeStep $powershell @("-NoProfile", "-ExecutionPolicy", "Bypass", "-File", $installer)
    } finally {
        $env:CODEX_NON_INTERACTIVE = $previousNonInteractive
        Remove-Item -LiteralPath $installer -Force -ErrorAction SilentlyContinue
    }
}

function Update-Codex([string]$Manager) {
    switch ($Manager) {
        "npm" { Invoke-NativeStep "npm" @("install", "-g", "@openai/codex@latest") }
        "bun" { Invoke-NativeStep "bun" @("add", "-g", "@openai/codex@latest") }
        default {
            Write-Step "Running the official Codex installer"
            Invoke-OfficialInstaller
        }
    }
}

# Windows PowerShell 5.1 默认可能未启用 TLS 1.2
if ($PSVersionTable.PSVersion.Major -lt 6) {
    [System.Net.ServicePointManager]::SecurityProtocol = [System.Net.ServicePointManager]::SecurityProtocol -bor [System.Net.SecurityProtocolType]::Tls12
}

if ($BaseUrl -notmatch "^https?://.+") {
    throw "-BaseUrl must be an http:// or https:// gateway address."
}
$BaseUrl = $BaseUrl.TrimEnd("/")
if ([string]::IsNullOrEmpty($ApiKey)) {
    throw "-ApiKey is required."
}
$baseUrlToml = ConvertTo-TomlString $BaseUrl "-BaseUrl"
$modelCatalogUrlToml = ConvertTo-TomlString "$BaseUrl/models" "-BaseUrl"
$apiKeyToml = ConvertTo-TomlString $ApiKey "-ApiKey"

$latestVersion = Get-LatestCodexVersion
$codexPath = Resolve-Codex
$currentVersion = $null
if ($null -eq $codexPath) {
    Write-Step "Codex CLI not found"
    $manager = "standalone"
} else {
    $manager = Get-CodexManager $codexPath
    $currentVersion = Get-CodexVersion $codexPath
    $shownVersion = if ($currentVersion) { $currentVersion } else { "of unknown version" }
    Write-Step "Found Codex $shownVersion at $codexPath ($manager)"
}

if ($latestVersion -and $currentVersion -eq $latestVersion) {
    Write-Step "Codex $currentVersion is already the latest release"
} else {
    if ($latestVersion) {
        Write-Step "Updating Codex to $latestVersion"
    } else {
        Write-WarningStep "Could not read the latest Codex release. Updating with $manager anyway."
    }
    try {
        Update-Codex $manager
    } catch {
        if ($null -eq $codexPath) {
            throw "Codex installation failed: $($_.Exception.Message) Install it with: irm $CodexInstallerUrl | iex"
        }
        Write-WarningStep "Codex update failed: $($_.Exception.Message) Continuing with the installed version."
    }
    $codexPath = Resolve-Codex
    if ($null -eq $codexPath) {
        throw "Codex was installed but could not be found. Open a new terminal and run this command again."
    }
    $currentVersion = Get-CodexVersion $codexPath
    # 多份安装并存时 PATH 顺序决定实际运行的版本
    if ($latestVersion -and $currentVersion -ne $latestVersion) {
        $shownVersion = if ($currentVersion) { $currentVersion } else { "an unknown version" }
        Write-WarningStep "$codexPath still reports $shownVersion. Check PATH for another Codex install."
    }
}

$codexHome = if ([string]::IsNullOrWhiteSpace($env:CODEX_HOME)) { Join-Path $env:USERPROFILE ".codex" } else { $env:CODEX_HOME }
$configPath = Join-Path $codexHome "config.toml"
New-Item -ItemType Directory -Force -Path $codexHome | Out-Null
if (Test-Path -LiteralPath $configPath) {
    $backupPath = "$configPath.bak." + (Get-Date -Format "yyyyMMddHHmmss")
    Copy-Item -LiteralPath $configPath -Destination $backupPath
    Write-Step "Backed up the existing config to $backupPath"
}

# 一次替换全部占位符，Key 或地址中出现的占位符文本不会被再次展开
$configValues = @{
    BASE_URL = $baseUrlToml
    MODEL_CATALOG_URL = $modelCatalogUrlToml
    WEBSOCKET_ENABLED = $(if ($WebSocket) { "true" } else { "false" })
    API_KEY = $apiKeyToml
}
$config = [System.Text.RegularExpressions.Regex]::Replace($ConfigTemplate, "@@([A-Z_]+)@@", {
    param($match)
    return $configValues[$match.Groups[1].Value]
})
$temporaryConfig = "$configPath.tmp.$PID"
try {
    # Codex 按 UTF-8 读取配置，Windows PowerShell 的 UTF8 编码会写入 BOM，改用无 BOM 编码
    [System.IO.File]::WriteAllText($temporaryConfig, $config + "`n", (New-Object System.Text.UTF8Encoding $false))
    Move-Item -LiteralPath $temporaryConfig -Destination $configPath -Force
} finally {
    Remove-Item -LiteralPath $temporaryConfig -Force -ErrorAction SilentlyContinue
}

$shownVersion = if ($currentVersion) { $currentVersion } else { "(unknown version)" }
Write-Step "Wrote $configPath"
Write-Step "Codex $shownVersion now uses $BaseUrl"
Write-Step "Quit running Codex sessions, then start a new one: codex"
