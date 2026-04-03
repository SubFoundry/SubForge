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
    & $Action
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
    cargo audit
}

Invoke-Step -Name "pnpm audit (high+)" -Action {
    pnpm audit --audit-level=high
}

Invoke-Step -Name "subforge-core release 构建" -Action {
    cargo build -p subforge-core --release
}

Invoke-Step -Name "desktop tauri no-bundle 构建冒烟" -Action {
    pnpm -C apps/desktop tauri build --ci --no-bundle
}

Write-Host "P8.7 Gate 校验通过。"
