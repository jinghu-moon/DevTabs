$global:DevTabsControl = '__CONTROL__'
$global:DevTabsSource = ConvertFrom-Json -InputObject '__SOURCE_JSON__'
function global:DevTabs-State([string]$state, [int]$code = 0) {
    $data = @{ pid = $PID; state = $state; code = $code } | ConvertTo-Json -Compress
    $tmp = $global:DevTabsControl + '.tmp'
    [IO.File]::WriteAllText($tmp, $data, [Text.UTF8Encoding]::new($false))
    [IO.File]::Move($tmp, $global:DevTabsControl, $true)
}
function global:DevTabs-Run {
    if ([string]::IsNullOrWhiteSpace($global:DevTabsSource)) { DevTabs-State 'idle'; return }
    DevTabs-State 'running'
    $global:LASTEXITCODE = 0
    try {
        & ([scriptblock]::Create($global:DevTabsSource))
        $ok = $?
        $code = if ($ok) { $global:LASTEXITCODE } elseif ($global:LASTEXITCODE) { $global:LASTEXITCODE } else { 1 }
    } catch { Write-Error $_; $code = 1 }
    finally { if ($null -eq $code) { $code = 130 }; DevTabs-State 'exited' $code }
}
$global:DevTabsOriginalPrompt = (Get-Command prompt).ScriptBlock
if (Get-Command Set-PSReadLineKeyHandler -ErrorAction Ignore) {
    Set-PSReadLineKeyHandler -Chord 'Ctrl+Alt+F12' -ScriptBlock {
        # Cancel the current edit buffer before accepting a managed request.
        # The private key is handled by PSReadLine, never as an Enter that
        # could accidentally submit a partially typed user command.
        [Microsoft.PowerShell.PSConsoleReadLine]::RevertLine()
        [Microsoft.PowerShell.PSConsoleReadLine]::AcceptLine()
    }
}
function global:prompt {
    $request = [IO.Path]::ChangeExtension($global:DevTabsControl, 'request')
    if ([IO.File]::Exists($request)) {
        try {
            $value = [IO.File]::ReadAllText($request) | ConvertFrom-Json
            [IO.File]::Delete($request)
            if ($null -ne $value.source) { $global:DevTabsSource = $value.source }
            DevTabs-Run
        } catch { Write-Error $_ }
    }
    & $global:DevTabsOriginalPrompt
}
DevTabs-State 'idle'
DevTabs-Run
