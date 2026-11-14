#requires -Version 7.0
<#
.SYNOPSIS
    Interactive Windows Terminal project launcher.
.DESCRIPTION
    Scans immediate child directories under a project root. Select a project
    with arrow keys / search, then opens one Windows Terminal tab with two
    PowerShell panes: Agent (left) and Debug (right).
#>

[CmdletBinding()]
param(
    [string] $RootPath = 'D:\100_Projects\110_Daily'
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function Get-ProjectDirectories {
    param([Parameter(Mandatory)][string] $Path)

    if (-not (Test-Path -LiteralPath $Path -PathType Container)) {
        throw "项目根目录不存在：$Path"
    }

    @(Get-ChildItem -LiteralPath $Path -Directory -Force |
        Where-Object { -not ($_.Attributes -band [IO.FileAttributes]::ReparsePoint) } |
        Sort-Object -Property Name)
}

function Write-Launcher {
    param(
        [string[]] $Projects,
        [string] $Filter,
        [int] $SelectedIndex,
        [string] $Root
    )

    $matches = @($Projects | Where-Object { $_ -like "*$Filter*" })
    if ($matches.Count -eq 0) { $SelectedIndex = 0 }
    elseif ($SelectedIndex -ge $matches.Count) { $SelectedIndex = $matches.Count - 1 }

    Clear-Host
    Write-Host ''
    Write-Host '  PROJECT LAUNCHER' -ForegroundColor Cyan
    Write-Host '  ─────────────────────────────────────────────────────' -ForegroundColor DarkGray
    Write-Host "  Root    $Root" -ForegroundColor DarkGray
    Write-Host "  Search  $Filter" -ForegroundColor Yellow
    Write-Host ''

    if ($matches.Count -eq 0) {
        Write-Host '  没有匹配的项目。' -ForegroundColor DarkYellow
    }
    else {
        $visibleCount = [Math]::Min(14, $matches.Count)
        $start = [Math]::Max(0, [Math]::Min($SelectedIndex - 6, $matches.Count - $visibleCount))
        $end = [Math]::Min($matches.Count - 1, $start + $visibleCount - 1)

        for ($i = $start; $i -le $end; $i++) {
            $name = $matches[$i]
            if ($i -eq $SelectedIndex) {
                Write-Host ("  > {0}" -f $name.PadRight(42)) -ForegroundColor Black -BackgroundColor Cyan
            }
            else {
                Write-Host ("    {0}" -f $name)
            }
        }

        Write-Host ''
        Write-Host ("  {0} 个项目" -f $matches.Count) -ForegroundColor DarkGray
    }

    Write-Host ''
    Write-Host '  ↑ / ↓ 选择   Enter 打开   Esc 退出   F5 刷新' -ForegroundColor DarkGray
    Write-Host '  直接输入文字即可搜索，Backspace 删除字符' -ForegroundColor DarkGray

    return $matches
}

function Start-ProjectWorkspace {
    param([Parameter(Mandatory)][IO.DirectoryInfo] $Project)

    $wt = Get-Command 'wt.exe' -ErrorAction SilentlyContinue
    if (-not $wt) {
        throw '找不到 wt.exe。请确认已安装 Windows Terminal，且 wt.exe 位于 PATH。'
    }

    $projectPath = $Project.FullName
    $projectName = $Project.Name

    # Keep the path and title safe when embedded in a PowerShell -Command string.
    $quotedPath = $projectPath.Replace("'", "''")
    $quotedAgentTitle = "$projectName · Agent".Replace("'", "''")
    $quotedDebugTitle = "$projectName · Debug".Replace("'", "''")

    # The role variable lets the Starship-compatible prompt title hook retain
    # the Agent / Debug suffix after each prompt is rendered.
    $agentCommand = "Set-Location -LiteralPath '$quotedPath'; `$env:PROJECT_TERMINAL_ROLE = 'Agent'; `$host.UI.RawUI.WindowTitle = '$quotedAgentTitle'"
    $debugCommand = "Set-Location -LiteralPath '$quotedPath'; `$env:PROJECT_TERMINAL_ROLE = 'Debug'; `$host.UI.RawUI.WindowTitle = '$quotedDebugTitle'"

    $wtArgs = @(
        '-w', '0',
        'new-tab',
        '--title', "$projectName · Agent",
        '--startingDirectory', $projectPath,
        'pwsh.exe', '-NoLogo', '-NoExit', '-Command', $agentCommand,
        ';',
        'split-pane',
        '-V',
        '--title', "$projectName · Debug",
        '--startingDirectory', $projectPath,
        'pwsh.exe', '-NoLogo', '-NoExit', '-Command', $debugCommand
    )

    & $wt.Source @wtArgs
    if ($LASTEXITCODE -ne 0) {
        throw "Windows Terminal 启动失败，退出代码：$LASTEXITCODE"
    }
}

try {
    $projects = @(Get-ProjectDirectories -Path $RootPath)
    if ($projects.Count -eq 0) {
        Write-Host "项目根目录下没有子文件夹：$RootPath" -ForegroundColor Yellow
        return
    }

    $filter = ''
    $selectedIndex = 0
    $matches = @()
    [Console]::CursorVisible = $false

    try {
        while ($true) {
            $matches = @(Write-Launcher -Projects @($projects.Name) -Filter $filter -SelectedIndex $selectedIndex -Root $RootPath)
            $key = [Console]::ReadKey($true)

            switch ($key.Key) {
                'UpArrow' {
                    if ($matches.Count -gt 0) {
                        $selectedIndex = ($selectedIndex - 1 + $matches.Count) % $matches.Count
                    }
                }
                'DownArrow' {
                    if ($matches.Count -gt 0) {
                        $selectedIndex = ($selectedIndex + 1) % $matches.Count
                    }
                }
                'Enter' {
                    if ($matches.Count -gt 0) {
                        $selectedName = $matches[$selectedIndex]
                        $project = $projects | Where-Object Name -EQ $selectedName | Select-Object -First 1
                        Start-ProjectWorkspace -Project $project
                        return
                    }
                }
                'Escape' { return }
                'Backspace' {
                    if ($filter.Length -gt 0) {
                        $filter = $filter.Substring(0, $filter.Length - 1)
                        $selectedIndex = 0
                    }
                }
                'F5' {
                    $projects = @(Get-ProjectDirectories -Path $RootPath)
                    $selectedIndex = 0
                }
                default {
                    if (-not [char]::IsControl($key.KeyChar)) {
                        $filter += $key.KeyChar
                        $selectedIndex = 0
                    }
                }
            }
        }
    }
    finally {
        [Console]::CursorVisible = $true
        Clear-Host
    }
}
catch {
    [Console]::CursorVisible = $true
    Write-Host "Project Launcher 错误：$($_.Exception.Message)" -ForegroundColor Red
    throw
}
