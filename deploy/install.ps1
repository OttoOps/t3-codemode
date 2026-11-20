<#
.SYNOPSIS
Installs t3cm on Windows: the binary, a per-user logon task that runs it, and the Claude Code MCP entry.

.DESCRIPTION
By default it downloads the latest release from OttoOps/t3-codemode, checks it against the
release's SHA256SUMS and, with gh signed in, its build provenance. The task runs at logon as
the current user, so no administrator rights are needed.

Then it asks whether to use the T3 workflow, and on yes runs t3cm skills install, which writes
the release's workflow skills and applies their setup steps. Skills that npx skills or a copy
by hand installed are replaced (t3cm skills install --take-over) unless you say no. A rerun
brings skills t3cm installed to the new release without asking. -Workflow and -NoWorkflow
answer for unattended runs; with no console to ask on, the answer is yes.

.EXAMPLE
powershell -ExecutionPolicy Bypass -File install.ps1
powershell -ExecutionPolicy Bypass -File install.ps1 -Version v0.4.0
powershell -ExecutionPolicy Bypass -File install.ps1 -Archive t3cm-0.4.0-x86_64-pc-windows-msvc.zip -Sums SHA256SUMS
powershell -ExecutionPolicy Bypass -File install.ps1 -NoWorkflow
powershell -ExecutionPolicy Bypass -File install.ps1 -Uninstall
#>
[CmdletBinding(DefaultParameterSetName = 'Release')]
param(
    [Parameter(ParameterSetName = 'Release')]
    [string]$Version,
    [Parameter(ParameterSetName = 'Archive', Mandatory = $true)]
    [string]$Archive,
    [Parameter(ParameterSetName = 'Archive', Mandatory = $true)]
    [string]$Sums,
    [Parameter(ParameterSetName = 'Uninstall', Mandatory = $true)]
    [switch]$Uninstall,
    [Parameter(ParameterSetName = 'Release')]
    [Parameter(ParameterSetName = 'Archive')]
    [switch]$Workflow,
    [Parameter(ParameterSetName = 'Release')]
    [Parameter(ParameterSetName = 'Archive')]
    [switch]$NoWorkflow,
    [switch]$FilesOnly
)

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
Set-StrictMode -Version Latest

$Mode = $PSCmdlet.ParameterSetName
if ($Workflow -and $NoWorkflow) {
    throw 'install.ps1: -Workflow and -NoWorkflow cannot be combined'
}
$Repo = 'OttoOps/t3-codemode'
$Releases = if ($env:T3CM_RELEASES_URL) { $env:T3CM_RELEASES_URL } else { "https://github.com/$Repo/releases" }
$Target = 'x86_64-pc-windows-msvc'
$TaskName = 't3-codemode'
$InstallDir = Join-Path $env:LOCALAPPDATA 'Programs\t3-codemode'
$Exe = Join-Path $InstallDir 't3cm.exe'
$StoreDir = Join-Path $env:LOCALAPPDATA 't3-codemode'
$Log = Join-Path $StoreDir 'logs\t3cm.log'
$McpName = 't3cm'
$McpAdd = @('mcp', 'add', '--transport', 'http', '--scope', 'user', $McpName, 'http://127.0.0.1:47731/mcp',
    '--header', 'X-T3-Thread: ${T3CODE_THREAD_ID}', '--header', 'X-T3-Config-Dir: ${CLAUDE_CONFIG_DIR:-}')

function Fail([string]$Message) {
    throw "install.ps1: $Message"
}

function Invoke-Native([string]$Command, [string[]]$Arguments) {
    $saved = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    $global:LASTEXITCODE = $null
    try {
        & $Command @Arguments *> $null
    } finally {
        $ErrorActionPreference = $saved
    }
    return $LASTEXITCODE
}

function Get-LatestTag {
    try {
        $response = Invoke-WebRequest -Uri "$Releases/latest" -Method Head -UseBasicParsing
    } catch {
        Fail "could not reach $Releases/latest: $($_.Exception.Message)"
    }
    $base = $response.BaseResponse
    $uri = if ($base.PSObject.Properties['ResponseUri']) { $base.ResponseUri } else { $base.RequestMessage.RequestUri }
    $tag = $uri.Segments[-1]
    if ($tag -notmatch '^v[0-9]') {
        Fail "no published release at $Releases yet"
    }
    return $tag
}

function Get-File([string]$Url, [string]$Path) {
    try {
        Invoke-WebRequest -Uri $Url -OutFile $Path -UseBasicParsing
    } catch {
        Fail "download failed: $Url ($($_.Exception.Message))"
    }
}

function Test-Checksum([string]$File, [string]$SumsFile) {
    $name = Split-Path $File -Leaf
    $want = $null
    foreach ($line in Get-Content -LiteralPath $SumsFile) {
        if ($line -match '^([0-9a-fA-F]{64}) [ *](.+)$' -and $Matches[2] -eq $name) {
            $want = $Matches[1].ToLowerInvariant()
            break
        }
    }
    if (-not $want) {
        Fail "$name is not listed in $SumsFile"
    }
    $got = (Get-FileHash -Algorithm SHA256 -LiteralPath $File).Hash.ToLowerInvariant()
    if ($got -ne $want) {
        Fail "checksum mismatch for ${name}: SHA256SUMS says $want, the file is $got"
    }
    Write-Host "Checksum OK: $name"
}

function Test-Provenance([string]$File) {
    if (-not (Get-Command gh -ErrorAction SilentlyContinue) -or (Invoke-Native gh @('auth', 'status')) -ne 0) {
        Write-Host 'gh is missing or not signed in; skipped the provenance check (gh attestation verify).'
        return
    }
    if ((Invoke-Native gh @('attestation', 'verify', $File, '-R', $Repo)) -ne 0) {
        Fail "gh attestation verify failed for $(Split-Path $File -Leaf)"
    }
    Write-Host "Provenance OK: $(Split-Path $File -Leaf) has a build attestation from $Repo"
}

function Stop-T3cm {
    if (Get-ScheduledTask -TaskName $TaskName -ErrorAction SilentlyContinue) {
        Disable-ScheduledTask -TaskName $TaskName | Out-Null
        Stop-ScheduledTask -TaskName $TaskName
    }
    for ($i = 0; $i -lt 50; $i++) {
        $running = @(Get-Process -Name t3cm -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $Exe })
        if ($running.Count -eq 0) {
            return
        }
        if ($i -eq 25) {
            $running | Stop-Process -Force
        }
        Start-Sleep -Milliseconds 200
    }
    Fail "t3cm is still running from $Exe"
}

function Register-T3cm {
    $user = "$env:USERDOMAIN\$env:USERNAME"
    $action = New-ScheduledTaskAction -Execute $Exe -Argument "serve --log-file `"$Log`"" -WorkingDirectory $InstallDir
    $atLogon = New-ScheduledTaskTrigger -AtLogOn -User $user
    $every5Minutes = New-ScheduledTaskTrigger -Once -At (Get-Date) -RepetitionInterval (New-TimeSpan -Minutes 5)
    $principal = New-ScheduledTaskPrincipal -UserId $user -LogonType Interactive -RunLevel Limited
    $settings = New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries -StartWhenAvailable `
        -ExecutionTimeLimit ([TimeSpan]::Zero) -MultipleInstances IgnoreNew `
        -RestartCount 3 -RestartInterval (New-TimeSpan -Minutes 1)
    Register-ScheduledTask -TaskName $TaskName -Action $action -Trigger @($atLogon, $every5Minutes) -Principal $principal -Settings $settings `
        -Description 't3-codemode: Code Mode MCP server for T3 Code (t3cm serve)' -Force | Out-Null
}

function Register-Mcp {
    if (-not (Get-Command claude -ErrorAction SilentlyContinue)) {
        Write-Host 'claude is not on PATH; register the MCP server yourself:'
        Write-Host "  claude $(($McpAdd | ForEach-Object { if ($_ -match ' ') { "'$_'" } else { $_ } }) -join ' ')"
        return
    }
    if ((Invoke-Native claude @('mcp', 'get', $McpName)) -eq 0) {
        Write-Host "Claude Code already has an MCP server named $McpName; left as is (t3cm skills update adds X-T3-Config-Dir to it)."
        return
    }
    & claude @McpAdd
    if ($LASTEXITCODE -ne 0) {
        Fail 'claude mcp add failed'
    }
}

function Get-SkillsState {
    $saved = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    try {
        $json = (& $Exe skills status --json 2> $null) -join "`n"
    } finally {
        $ErrorActionPreference = $saved
    }
    try {
        return $json | ConvertFrom-Json
    } catch {
        return $null
    }
}

function Test-CanAsk {
    if (-not [Environment]::UserInteractive -or [Console]::IsInputRedirected) {
        return $false
    }
    return -not ([Environment]::GetCommandLineArgs() | Where-Object { $_ -match '^-noni' })
}

function Confirm-Yes([string]$Question) {
    if (-not (Test-CanAsk)) {
        Write-Host "$Question Yes: there is no console to ask on (-NoWorkflow says no)."
        return $true
    }
    try {
        $answer = Read-Host "$Question [Y/n]"
    } catch {
        $answer = ''
    }
    return $answer -notmatch '^\s*n'
}

function Install-Workflow {
    $state = Get-SkillsState
    if ($state -and $state.installed) {
        $cmd = @('update')
    } elseif ($state -and $state.foreign) {
        $cmd = @('install', '--take-over')
        $question = "npx skills or a copy by hand installed the T3 workflow skills here. Replace them with this release's, and have t3cm keep them current?"
    } else {
        $cmd = @('install')
        $question = 'Use the T3 workflow?'
    }
    $leave = "Left the T3 workflow skills alone; to have t3cm manage them, run: & '$Exe' skills $($cmd -join ' ')"
    if ($NoWorkflow) {
        Write-Host $leave
        return $true
    }
    if (-not $Workflow) {
        if ($cmd[0] -eq 'update') {
            Write-Host 't3cm installed the T3 workflow skills here, so this install brings them to its release (-NoWorkflow skips that).'
        } elseif (-not (Confirm-Yes $question)) {
            Write-Host $leave
            return $true
        }
    }
    & $Exe skills @cmd | Out-Host
    return $LASTEXITCODE -eq 0
}

function Install-T3cm {
    $work = Join-Path ([IO.Path]::GetTempPath()) ("t3cm-install-" + [Guid]::NewGuid())
    New-Item -ItemType Directory -Path $work | Out-Null
    try {
        if ($Mode -eq 'Archive') {
            $zip = (Resolve-Path -LiteralPath $Archive).Path
            $sumsFile = (Resolve-Path -LiteralPath $Sums).Path
        } else {
            if ($Releases -notmatch '^(https://|http://(127\.0\.0\.1|localhost)(:[0-9]+)?(/|$))') {
                Fail "T3CM_RELEASES_URL must be https (plain http only on 127.0.0.1 or localhost): $Releases"
            }
            $tag = if ($Version) { $Version } else { Get-LatestTag }
            if (-not $tag.StartsWith('v')) {
                $tag = "v$tag"
            }
            $name = "t3cm-$($tag.Substring(1))-$Target.zip"
            $zip = Join-Path $work $name
            $sumsFile = Join-Path $work 'SHA256SUMS'
            Get-File "$Releases/download/$tag/$name" $zip
            Get-File "$Releases/download/$tag/SHA256SUMS" $sumsFile
        }
        Test-Checksum $zip $sumsFile
        if ($Mode -eq 'Archive') {
            Write-Host "Local archive: provenance not checked; run gh attestation verify $zip -R $Repo for a release download."
        } else {
            Test-Provenance $zip
        }
        $name = Split-Path $zip -Leaf
        if ($name -notmatch '^t3cm-.+\.zip$') {
            Fail "$name is not a t3cm Windows release archive (t3cm-<version>-$Target.zip)"
        }
        Expand-Archive -LiteralPath $zip -DestinationPath (Join-Path $work 'x')
        $new = Join-Path $work "x\$($name -replace '\.zip$', '')\t3cm.exe"
        if (-not (Test-Path -LiteralPath $new)) {
            Fail "$name has no t3cm.exe in its top-level folder"
        }
        if ((Invoke-Native $new @('--version')) -ne 0) {
            Fail "the t3cm.exe in $name does not run on this machine"
        }

        New-Item -ItemType Directory -Force -Path $InstallDir, (Split-Path $Log) | Out-Null
        Copy-Item -LiteralPath $new -Destination "$Exe.new" -Force
        $wasEnabled = $false
        if (-not $FilesOnly) {
            $existing = Get-ScheduledTask -TaskName $TaskName -ErrorAction SilentlyContinue
            $wasEnabled = $existing -and $existing.State -ne 'Disabled'
        }
        try {
            if (-not $FilesOnly) {
                Stop-T3cm
            }
            Move-Item -LiteralPath "$Exe.new" -Destination $Exe -Force
            if ($FilesOnly) {
                Write-Host "Installed $Exe; no logon task registered (-FilesOnly)."
            } else {
                Register-T3cm
                Start-ScheduledTask -TaskName $TaskName
            }
        } catch {
            if ($wasEnabled) {
                Enable-ScheduledTask -TaskName $TaskName | Out-Null
                Start-ScheduledTask -TaskName $TaskName -ErrorAction SilentlyContinue
            }
            throw
        }
    } finally {
        Remove-Item -LiteralPath $work -Recurse -Force -ErrorAction SilentlyContinue
    }
    $skillsOk = Install-Workflow
    Register-Mcp
    $installed = (& $Exe --version) -replace '^t3cm ', ''
    Write-Host "t3-codemode $installed installed. Check it with: & '$Exe' status   Logs: $Log"
    if (-not $skillsOk) {
        Fail "t3cm is installed, but its workflow skills are not; fix what t3cm skills install printed, then run: & '$Exe' skills install"
    }
}

function Uninstall-T3cm {
    if (-not $FilesOnly) {
        Stop-T3cm
        if (Get-ScheduledTask -TaskName $TaskName -ErrorAction SilentlyContinue) {
            Unregister-ScheduledTask -TaskName $TaskName -Confirm:$false
        }
    }
    if (Test-Path -LiteralPath $InstallDir) {
        try {
            Remove-Item -LiteralPath $InstallDir -Recurse -Force
        } catch {
            Fail "could not remove ${InstallDir}: $($_.Exception.Message). Stop t3cm.exe and run -Uninstall again."
        }
    }
    if (Get-Command claude -ErrorAction SilentlyContinue) {
        if ((Invoke-Native claude @('mcp', 'get', $McpName)) -eq 0 -and
            (Invoke-Native claude @('mcp', 'remove', '--scope', 'user', $McpName)) -ne 0) {
            Write-Warning "could not remove the $McpName MCP entry; retry with: claude mcp remove --scope user $McpName"
        }
    } else {
        Write-Host "claude is not on PATH; remove the MCP entry with: claude mcp remove --scope user $McpName"
    }
    Write-Host "t3-codemode removed. Its store and logs stay in $StoreDir; delete it to drop memory and links."
}

if ($Uninstall) {
    Uninstall-T3cm
} else {
    Install-T3cm
}
