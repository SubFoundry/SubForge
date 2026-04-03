Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Invoke-Step {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Name,
        [Parameter(Mandatory = $true)]
        [scriptblock]$Action
    )

    Write-Host "==> $Name"
    $startedAt = Get-Date
    $global:LASTEXITCODE = 0
    & $Action
    if (-not $?) {
        throw "步骤 '$Name' 执行失败。"
    }
    if ($LASTEXITCODE -ne 0) {
        throw "步骤 '$Name' 执行失败，退出码：$LASTEXITCODE"
    }
    $elapsed = (Get-Date) - $startedAt
    Write-Host ("    完成，耗时 {0:n1}s" -f $elapsed.TotalSeconds)
}

function Ensure-CargoAudit {
    if (Get-Command cargo-audit -ErrorAction SilentlyContinue) {
        return
    }

    Write-Host "cargo-audit 未安装，正在执行一次性安装..."
    cargo install cargo-audit --locked
}

function Ensure-RustTarget {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Target
    )

    $installedTargets = rustup target list --installed
    if ($installedTargets -contains $Target) {
        return
    }

    Write-Host "Rust target '$Target' 未安装，正在安装..."
    rustup target add $Target
}

function Test-IsLinux {
    return [System.Runtime.InteropServices.RuntimeInformation]::IsOSPlatform(
        [System.Runtime.InteropServices.OSPlatform]::Linux
    )
}

Invoke-Step -Name "workspace cargo check" -Action {
    cargo check --workspace
}

Invoke-Step -Name "workspace cargo test" -Action {
    cargo test --workspace
}

Invoke-Step -Name "workspace cargo clippy (deny warnings)" -Action {
    cargo clippy --workspace --all-targets -- -D warnings
}

Invoke-Step -Name "ensure cargo-audit is available" -Action {
    Ensure-CargoAudit
}

Invoke-Step -Name "cargo audit" -Action {
    cargo audit --stale
}

Invoke-Step -Name "pnpm audit (high+)" -Action {
    pnpm audit --audit-level=high --registry=https://registry.npmjs.org
}

Invoke-Step -Name "subforge-core release 构建" -Action {
    cargo build -p subforge-core --release
}

if (Test-IsLinux) {
    Invoke-Step -Name "ensure linux musl target is available" -Action {
        Ensure-RustTarget -Target "x86_64-unknown-linux-musl"
    }

    Invoke-Step -Name "subforge-core release 构建（linux musl）" -Action {
        cargo build -p subforge-core --release --target x86_64-unknown-linux-musl
    }
}
else {
    Write-Host "当前非 Linux 环境，跳过 musl 目标构建校验。"
}

Invoke-Step -Name "desktop tauri no-bundle 构建冒烟" -Action {
    pnpm -C apps/desktop tauri build --ci --no-bundle
}

Write-Host "Release Gate 校验通过。"
