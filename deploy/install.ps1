<#
.SYNOPSIS
Installs t3cm on Windows: the binary, a per-user logon task that runs it, and the Claude Code and Antigravity MCP entries.

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
$T3Home = if ($env:T3CODE_HOME) { $env:T3CODE_HOME } else { Join-Path $env:USERPROFILE '.t3' }
$AntigravityDir = Join-Path $T3Home 'userdata\providers\antigravity'
$AntigravityCaveat = "Note: adding $McpName to Antigravity's mcp_config.json stops T3 Code from using Antigravity as its system text-generation model (for titles and similar tasks). Antigravity agent threads still work; select another system model in T3 Code if you use Antigravity there."

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

function Write-Utf8NoBom([string]$Path, [string]$Content) {
    $tmp = "$Path.t3cm-tmp"
    $utf8NoBom = New-Object System.Text.UTF8Encoding($false)
    try {
        [IO.File]::WriteAllText($tmp, $Content, $utf8NoBom)
        Move-Item -LiteralPath $tmp -Destination $Path -Force
    } catch {
        Remove-Item -LiteralPath $tmp -Force -ErrorAction SilentlyContinue
        throw
    }
}

function New-JsonDict {
    return (New-Object 'System.Collections.Generic.Dictionary[string, object]')
}

function ConvertFrom-CaseSensitiveJsonObject([string]$Raw) {
    $text = $Raw
    if ($text.Length -gt 0 -and $text[0] -eq [char]0xFEFF) {
        $text = $text.Substring(1)
    }
    if ([string]::IsNullOrWhiteSpace($text)) {
        return (New-JsonDict)
    }
    if ($PSVersionTable.PSVersion.Major -lt 6) {
        Add-Type -AssemblyName System.Web.Extensions
        $ser = New-Object System.Web.Script.Serialization.JavaScriptSerializer
        $ser.MaxJsonLength = [int]::MaxValue
        $parsed = $ser.DeserializeObject($text)
        if (-not (Test-JsonDict $parsed)) {
            throw 'top level must be a JSON object'
        }
        return $parsed
    }
    $jdoc = [System.Text.Json.JsonDocument]::Parse($text)
    try {
        if ($jdoc.RootElement.ValueKind -ne [System.Text.Json.JsonValueKind]::Object) {
            throw 'top level must be a JSON object'
        }
        function Convert-JsonElement([System.Text.Json.JsonElement]$El) {
            switch ($El.ValueKind) {
                'Object' {
                    $d = New-JsonDict
                    foreach ($p in $El.EnumerateObject()) {
                        $d[$p.Name] = Convert-JsonElement $p.Value
                    }
                    return $d
                }
                'Array' {
                    $list = New-Object System.Collections.ArrayList
                    foreach ($item in $El.EnumerateArray()) {
                        [void]$list.Add((Convert-JsonElement $item))
                    }
                    return ,($list.ToArray())
                }
                'String' { return $El.GetString() }
                'Number' {
                    $l = [int64]0
                    if ($El.TryGetInt64([ref]$l)) { return $l }
                    return $El.GetDouble()
                }
                'True' { return $true }
                'False' { return $false }
                default { return $null }
            }
        }
        return (Convert-JsonElement $jdoc.RootElement)
    } finally {
        $jdoc.Dispose()
    }
}

function Get-JsonDepth([object]$Node, [int]$Current) {
    if ($Current -gt 100) {
        return $Current
    }
    $max = $Current
    if (Test-JsonDict $Node) {
        foreach ($v in $Node.Values) {
            $d = Get-JsonDepth $v ($Current + 1)
            if ($d -gt $max) { $max = $d }
            if ($max -gt 100) { return $max }
        }
    } elseif ($Node -is [System.Array]) {
        foreach ($v in $Node) {
            $d = Get-JsonDepth $v ($Current + 1)
            if ($d -gt $max) { $max = $d }
            if ($max -gt 100) { return $max }
        }
    }
    return $max
}

function ConvertTo-LosslessJson([object]$Doc) {
    if ((Get-JsonDepth $Doc 0) -gt 99) {
        throw 'JSON nesting depth exceeds 99 levels'
    }
    return (($Doc | ConvertTo-Json -Depth 100) + "`n")
}

function Test-JsonDict([object]$Obj) {
    return ($null -ne $Obj -and $Obj -is [System.Collections.Generic.IDictionary[string, object]])
}

function Test-T3cmStdioEntry([object]$Entry) {
    if (-not (Test-JsonDict $Entry)) {
        return $false
    }
    if (-not $Entry.ContainsKey('command') -or -not $Entry.ContainsKey('args')) {
        return $false
    }
    $cmd = $Entry['command']
    $argsVal = $Entry['args']
    if ($cmd -isnot [string] -or $argsVal -isnot [System.Array] -or $argsVal.Count -lt 1 -or $argsVal[0] -cne 'stdio') {
        return $false
    }
    $base = ($cmd -replace '\\', '/').Split('/')[-1]
    return ($base -ceq $McpName -or $base -ieq 't3cm.exe')
}

function Test-T3cmAntigravityHelper {
    return ((Test-Path -LiteralPath $Exe -PathType Leaf) -and (Invoke-Native $Exe @('antigravity-mcp', '--help')) -eq 0)
}

function Get-AntigravityConfigs {
    if (-not (Test-Path -LiteralPath $AntigravityDir -PathType Container)) {
        return @()
    }
    $configs = @()
    foreach ($profile in @(Get-ChildItem -LiteralPath $AntigravityDir -Directory -ErrorAction SilentlyContinue | Sort-Object Name)) {
        $configDir = Join-Path $profile.FullName 'config'
        if (Test-Path -LiteralPath $configDir -PathType Container) {
            $configs += Join-Path $configDir 'mcp_config.json'
        }
    }
    return $configs
}

function Register-AntigravityMcp {
    if (-not (Test-Path -LiteralPath $AntigravityDir -PathType Container)) {
        return
    }
    if (Test-T3cmAntigravityHelper) {
        & $Exe antigravity-mcp --bin $Exe | Out-Host
        return
    }
    $wrote = $false
    foreach ($cfg in @(Get-AntigravityConfigs)) {
        try {
            if (Test-Path -LiteralPath $cfg -PathType Container) {
                Write-Warning "could not update Antigravity MCP config at $cfg (path is a directory); left as is."
                continue
            }
            $doc = $null
            if (Test-Path -LiteralPath $cfg -PathType Leaf) {
                $raw = [IO.File]::ReadAllText($cfg)
                $doc = ConvertFrom-CaseSensitiveJsonObject $raw
            } else {
                $doc = New-JsonDict
            }
            if (-not (Test-JsonDict $doc)) {
                Write-Warning "could not update Antigravity MCP config at $cfg (top level must be a JSON object); left as is."
                continue
            }
            if (-not $doc.ContainsKey('mcpServers')) {
                $doc['mcpServers'] = New-JsonDict
            } elseif (-not (Test-JsonDict $doc['mcpServers'])) {
                Write-Warning "could not update Antigravity MCP config at $cfg (mcpServers is not a JSON object); left as is."
                continue
            }
            $servers = $doc['mcpServers']
            $changed = $false
            if ($servers.ContainsKey($McpName)) {
                $existing = $servers[$McpName]
                if (-not (Test-T3cmStdioEntry $existing)) {
                    Write-Host "Antigravity already has an incompatible MCP server named $McpName in $cfg; left as is."
                    continue
                }
                if (-not $existing.ContainsKey('tools')) {
                    $existing['tools'] = New-JsonDict
                } elseif (-not (Test-JsonDict $existing['tools'])) {
                    Write-Warning "could not update Antigravity MCP config at $cfg (mcpServers.$McpName.tools is not a JSON object); left as is."
                    continue
                }
                $tools = $existing['tools']
                $badTool = $false
                foreach ($toolName in @('search', 'execute')) {
                    if (-not $tools.ContainsKey($toolName)) {
                        $tcfg = New-JsonDict
                        $tcfg['eager'] = $true
                        $tools[$toolName] = $tcfg
                        $changed = $true
                    } elseif (-not (Test-JsonDict $tools[$toolName])) {
                        Write-Warning "could not update Antigravity MCP config at $cfg (mcpServers.$McpName.tools.$toolName is not a JSON object); left as is."
                        $badTool = $true
                        break
                    } elseif (-not $tools[$toolName].ContainsKey('eager') -or $tools[$toolName]['eager'] -isnot [bool] -or -not $tools[$toolName]['eager']) {
                        $tools[$toolName]['eager'] = $true
                        $changed = $true
                    }
                }
                if ($badTool -or -not $changed) {
                    continue
                }
            } else {
                $searchCfg = New-JsonDict
                $searchCfg['eager'] = $true
                $execCfg = New-JsonDict
                $execCfg['eager'] = $true
                $toolsCfg = New-JsonDict
                $toolsCfg['search'] = $searchCfg
                $toolsCfg['execute'] = $execCfg
                $entry = New-JsonDict
                $entry['command'] = $Exe
                $entry['args'] = [object[]]@('stdio')
                $entry['tools'] = $toolsCfg
                $servers[$McpName] = $entry
                $changed = $true
            }
            if ($changed) {
                Write-Utf8NoBom $cfg (ConvertTo-LosslessJson $doc)
                Write-Host "Registered $McpName for Antigravity in $cfg"
                $wrote = $true
            }
        } catch {
            Write-Warning "could not update Antigravity MCP config at ${cfg}: $($_.Exception.Message); left as is."
        }
    }
    if ($wrote) {
        Write-Host $AntigravityCaveat
    }
}

function Unregister-AntigravityMcp {
    if (-not (Test-Path -LiteralPath $AntigravityDir -PathType Container)) {
        return
    }
    if (Test-T3cmAntigravityHelper) {
        & $Exe antigravity-mcp --remove | Out-Host
        return
    }
    foreach ($cfg in @(Get-AntigravityConfigs)) {
        if (-not (Test-Path -LiteralPath $cfg -PathType Leaf)) {
            continue
        }
        try {
            $raw = [IO.File]::ReadAllText($cfg)
            if ([string]::IsNullOrWhiteSpace($raw)) {
                continue
            }
            $doc = ConvertFrom-CaseSensitiveJsonObject $raw
            if (-not $doc.ContainsKey('mcpServers') -or -not (Test-JsonDict $doc['mcpServers'])) {
                continue
            }
            $servers = $doc['mcpServers']
            if (-not $servers.ContainsKey($McpName)) {
                continue
            }
            [void]$servers.Remove($McpName)
            Write-Utf8NoBom $cfg (ConvertTo-LosslessJson $doc)
            Write-Host "Removed $McpName for Antigravity from $cfg"
        } catch {
            Write-Warning "could not update Antigravity MCP config at ${cfg}: $($_.Exception.Message); left as is."
        }
    }
}

function Register-Mcp {
    if (-not (Get-Command claude -ErrorAction SilentlyContinue)) {
        Write-Host 'claude is not on PATH; register the MCP server yourself:'
        Write-Host "  claude $(($McpAdd | ForEach-Object { if ($_ -match ' ') { "'$_'" } else { $_ } }) -join ' ')"
    } elseif ((Invoke-Native claude @('mcp', 'get', $McpName)) -eq 0) {
        Write-Host "Claude Code already has an MCP server named $McpName; left as is (t3cm skills update adds X-T3-Config-Dir to it)."
    } else {
        & claude @McpAdd
        if ($LASTEXITCODE -ne 0) {
            Fail 'claude mcp add failed'
        }
    }
    Register-AntigravityMcp
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
    Unregister-AntigravityMcp
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
