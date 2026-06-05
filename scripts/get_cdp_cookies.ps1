param (
    [Parameter(Mandatory=$false, HelpMessage="The domain to fetch cookies for (e.g. platform.xiaomimimo.com)")]
    [string]$Domain = "",

    [ValidateSet("mimo", "opencode", "opencodego", "abacus", "alibabatoken", "t3chat", "amp", "cursor")]
    [string]$Provider = "",

    [int]$Port = 9222,

    [string]$ConfigPath = (Join-Path $env:APPDATA "quotify\quotify.toml"),

    [switch]$OpenChrome,

    [switch]$Interactive,

    [switch]$Sync
)

$ErrorActionPreference = "Stop"

$ProviderTargets = @(
    [pscustomobject]@{ Key = "mimo"; Provider = "mimo"; Field = "cookie_header"; Domain = "platform.xiaomimimo.com"; Aliases = @("mimo", "xiaomimimo.com", "www.xiaomimimo.com"); Url = "https://platform.xiaomimimo.com/console/balance" },
    [pscustomobject]@{ Key = "opencode"; Provider = "opencode"; Field = "auth_cookie"; Domain = "opencode.ai"; Aliases = @("opencode", "opencodego"); Url = "https://opencode.ai" },
    [pscustomobject]@{ Key = "opencodego"; Provider = "opencode"; Field = "auth_cookie"; Domain = "opencode.ai"; Aliases = @("opencodego", "opencode"); Url = "https://opencode.ai" },
    [pscustomobject]@{ Key = "abacus"; Provider = "abacus"; Field = "api_key"; Domain = "abacus.ai"; Aliases = @("abacus", "apps.abacus.ai"); Url = "https://apps.abacus.ai" },
    [pscustomobject]@{ Key = "alibabatoken"; Provider = "alibabatoken"; Field = "api_key"; Domain = "aliyun.com"; Aliases = @("alibabatoken", "bailian.console.aliyun.com"); Url = "https://bailian.console.aliyun.com" },
    [pscustomobject]@{ Key = "t3chat"; Provider = "t3chat"; Field = "api_key"; Domain = "t3.chat"; Aliases = @("t3chat"); Url = "https://t3.chat" },
    [pscustomobject]@{ Key = "amp"; Provider = "amp"; Field = "api_key"; Domain = "ampcode.com"; Aliases = @("amp", "ampcode"); Url = "https://ampcode.com/settings" },
    [pscustomobject]@{ Key = "cursor"; Provider = "cursor"; Field = "api_key"; Domain = "cursor.com"; Aliases = @("cursor", "cursor.sh", "www.cursor.com"); Url = "https://www.cursor.com/settings" }
)

function Resolve-ProviderTarget {
    param (
        [string]$ProviderName,
        [string]$TargetDomain
    )

    if (-not [string]::IsNullOrWhiteSpace($ProviderName)) {
        return $ProviderTargets | Where-Object { $_.Key -eq $ProviderName } | Select-Object -First 1
    }

    $NormalizedDomain = $TargetDomain.ToLowerInvariant().Trim().TrimStart(".")
    foreach ($Target in $ProviderTargets) {
        $KnownNames = @($Target.Key, $Target.Domain) + $Target.Aliases
        foreach ($KnownName in $KnownNames) {
            $KnownDomain = $KnownName.ToLowerInvariant().TrimStart(".")
            if ($NormalizedDomain -eq $KnownDomain -or $NormalizedDomain.Contains($KnownDomain) -or $KnownDomain.Contains($NormalizedDomain)) {
                return $Target
            }
        }
    }

    return $null
}

function Read-ProviderTarget {
    Write-Host "Choose a cookie-based provider:" -ForegroundColor Cyan
    for ($i = 0; $i -lt $ProviderTargets.Count; $i++) {
        $Target = $ProviderTargets[$i]
        Write-Host ("  {0}. {1} ({2}) -> [{3}].{4}" -f ($i + 1), $Target.Key, $Target.Domain, $Target.Provider, $Target.Field)
    }

    while ($true) {
        $Choice = Read-Host "Provider number or domain"
        if ([string]::IsNullOrWhiteSpace($Choice)) {
            continue
        }

        $Choice = $Choice.Trim()
        $ChoiceNumber = 0
        if ([int]::TryParse($Choice, [ref]$ChoiceNumber) -and $ChoiceNumber -ge 1 -and $ChoiceNumber -le $ProviderTargets.Count) {
            return $ProviderTargets[$ChoiceNumber - 1]
        }

        $Target = Resolve-ProviderTarget -ProviderName "" -TargetDomain $Choice
        if ($Target) {
            return $Target
        }

        return [pscustomobject]@{ Key = ""; Provider = ""; Field = ""; Domain = $Choice; Url = "https://$Choice" }
    }
}

function ConvertTo-TomlString {
    param ([string]$Value)

    $Escaped = $Value.Replace("\", "\\").Replace('"', '\"')
    return "`"$Escaped`""
}

function Set-TomlField {
    param (
        [string]$Path,
        [string]$Section,
        [string]$Field,
        [string]$Value
    )

    $TomlValue = ConvertTo-TomlString $Value
    $SectionPattern = "^\s*\[$([regex]::Escape($Section))\]\s*$"
    $AnySectionPattern = "^\s*\[[^\]]+\]\s*$"
    $FieldPattern = "^\s*#?\s*$([regex]::Escape($Field))\s*="

    if (-not (Test-Path $Path)) {
        $Parent = Split-Path -Parent $Path
        if (-not [string]::IsNullOrWhiteSpace($Parent)) {
            New-Item -ItemType Directory -Path $Parent -Force | Out-Null
        }
        Set-Content -Path $Path -Encoding UTF8 -Value @("[$Section]", "$Field = $TomlValue")
        return
    }

    $Lines = [System.Collections.Generic.List[string]]::new()
    foreach ($Line in (Get-Content -Path $Path)) {
        $Lines.Add($Line)
    }

    $SectionStart = -1
    $SectionEnd = $Lines.Count
    for ($i = 0; $i -lt $Lines.Count; $i++) {
        if ($Lines[$i] -match $SectionPattern) {
            $SectionStart = $i
            for ($j = $i + 1; $j -lt $Lines.Count; $j++) {
                if ($Lines[$j] -match $AnySectionPattern) {
                    $SectionEnd = $j
                    break
                }
            }
            break
        }
    }

    if ($SectionStart -eq -1) {
        if ($Lines.Count -gt 0 -and -not [string]::IsNullOrWhiteSpace($Lines[$Lines.Count - 1])) {
            $Lines.Add("")
        }
        $Lines.Add("[$Section]")
        $Lines.Add("$Field = $TomlValue")
    } else {
        $Updated = $false
        for ($i = $SectionStart + 1; $i -lt $SectionEnd; $i++) {
            if ($Lines[$i] -match $FieldPattern) {
                $Lines[$i] = "$Field = $TomlValue"
                $Updated = $true
                break
            }
        }

        if (-not $Updated) {
            $Lines.Insert($SectionEnd, "$Field = $TomlValue")
        }
    }

    Set-Content -Path $Path -Encoding UTF8 -Value $Lines
}

function Open-DebugChrome {
    param (
        [string]$StartUrl,
        [int]$DebugPort
    )

    $ScriptDir = Split-Path -Parent $PSCommandPath
    $OpenScript = Join-Path $ScriptDir "open_debug_chrome.ps1"
    if (-not (Test-Path $OpenScript)) {
        throw "Cannot find $OpenScript"
    }

    & $OpenScript -Port $DebugPort -StartUrl $StartUrl
}

$InteractiveMode = $Interactive -or ([string]::IsNullOrWhiteSpace($Domain) -and [string]::IsNullOrWhiteSpace($Provider))
$ResolvedTarget = Resolve-ProviderTarget -ProviderName $Provider -TargetDomain $Domain

if ($InteractiveMode) {
    $ResolvedTarget = Read-ProviderTarget
    $Domain = $ResolvedTarget.Domain
    $OpenChrome = $true
    $Sync = $true
} elseif ([string]::IsNullOrWhiteSpace($Domain) -and $ResolvedTarget) {
    $Domain = $ResolvedTarget.Domain
}

if ([string]::IsNullOrWhiteSpace($Domain)) {
    throw "Domain is required. Pass -Domain platform.xiaomimimo.com, -Provider mimo, or run without arguments for interactive mode."
}

if (-not $ResolvedTarget) {
    $ResolvedTarget = Resolve-ProviderTarget -ProviderName "" -TargetDomain $Domain
}

# Normalize the target domain for filtering
$TargetDomain = $Domain.ToLowerInvariant().Trim().TrimStart('.')

if ($OpenChrome) {
    $StartUrl = if ($ResolvedTarget -and -not [string]::IsNullOrWhiteSpace($ResolvedTarget.Url)) { $ResolvedTarget.Url } else { "https://$TargetDomain" }
    Open-DebugChrome -StartUrl $StartUrl -DebugPort $Port
    Write-Host ""
    Write-Host "Log in to $StartUrl in the debug Chrome window, then press Enter to fetch cookies." -ForegroundColor Yellow
    Read-Host | Out-Null
}

# 1. Fetch active tabs from Chrome
$ListUrl = "http://localhost:$Port/json/list"
Write-Host "Fetching open tabs from Chrome at $ListUrl..." -ForegroundColor Gray
try {
    $Tabs = Invoke-RestMethod -Uri $ListUrl -Method Get
} catch {
    Write-Error "Failed to connect to Chrome at $ListUrl. Make sure Chrome is running with remote debugging enabled."
    return
}

if (-not $Tabs) {
    Write-Error "No active tabs found in Chrome. Make sure you have at least one tab open."
    return
}

# Find the tab containing the target domain
$TargetTab = $Tabs | Where-Object { $_.url.ToLower().Contains($TargetDomain) -or $_.title.ToLower().Contains($TargetDomain) } | Select-Object -First 1

if (-not $TargetTab) {
    Write-Host "Tab for domain '$Domain' not found in active tabs list. Falling back to the first available page..." -ForegroundColor Yellow
    $TargetTab = $Tabs | Where-Object { $_.type -eq "page" } | Select-Object -First 1
}

if (-not $TargetTab) {
    Write-Error "No active page tabs found in Chrome."
    return
}

$WsUrl = $TargetTab.webSocketDebuggerUrl
if (-not $WsUrl) {
    Write-Error "Could not find webSocketDebuggerUrl for the target tab."
    return
}

Write-Host "Connecting to Page WebSocket for tab '$($TargetTab.title)' ($($TargetTab.url))" -ForegroundColor Gray

# 2. Connect to WebSocket
$WebSocket = New-Object System.Net.WebSockets.ClientWebSocket
$CancellationToken = New-Object System.Threading.CancellationTokenSource

try {
    $ConnectTask = $WebSocket.ConnectAsync((New-Object System.Uri($WsUrl)), $CancellationToken.Token)
    $ConnectTask.Wait()
} catch {
    Write-Error "Failed to connect to Chrome WebSocket at $WsUrl."
    return
}

# Global request ID counter
$Script:NextId = 100

# Helper to send JSON
function Send-CdpCommand($Id, $Method, $Params) {
    $Payload = @{
        id = $Id
        method = $Method
        params = $Params
    } | ConvertTo-Json -Depth 5
    
    $Buffer = [System.Text.Encoding]::UTF8.GetBytes($Payload)
    $ArraySegment = New-Object System.ArraySegment[byte] @(,$Buffer)
    
    $SendTask = $WebSocket.SendAsync($ArraySegment, [System.Net.WebSockets.WebSocketMessageType]::Text, $true, $CancellationToken.Token)
    $SendTask.Wait()
}

# Helper to receive response matching ID (handles fragmentation cleanly)
function Receive-CdpResponse($TargetId) {
    $Stream = New-Object System.IO.MemoryStream
    $Buffer = New-Object byte[] 65536  # 64KB read chunks
    
    while ($true) {
        $ArraySegment = New-Object System.ArraySegment[byte] @(,$Buffer)
        $ReceiveTask = $WebSocket.ReceiveAsync($ArraySegment, $CancellationToken.Token)
        $ReceiveTask.Wait()
        
        $Result = $ReceiveTask.Result
        $Stream.Write($Buffer, 0, $Result.Count)
        
        if ($Result.EndOfMessage) {
            $Bytes = $Stream.ToArray()
            $Msg = [System.Text.Encoding]::UTF8.GetString($Bytes)
            
            try {
                $Json = $Msg | ConvertFrom-Json
                if ($Json.id -eq $TargetId) {
                    $Stream.Close()
                    return $Msg
                }
            } catch {
                # Not complete JSON or mismatched ID, keep looping
            }
            
            # Reset stream for next message
            $Stream.SetLength(0)
        }
    }
}

# 3. Retrieve and Filter cookies
try {
    # Get cookies for the active Page target
    Write-Host "Fetching cookies from the page target..." -ForegroundColor Gray
    $CmdId = $Script:NextId++
    Send-CdpCommand $CmdId "Network.getCookies" @{}
    $ResponseJson = Receive-CdpResponse $CmdId
    
    $Response = $ResponseJson | ConvertFrom-Json
    $Cookies = $Response.result.cookies
    
    if (-not $Cookies) {
        Write-Host "No cookies found in the page context." -ForegroundColor Yellow
        return
    }

    # Filter cookies belonging to the domain or subdomains
    $FilteredCookies = $Cookies | Where-Object {
        $CookieDomain = $_.domain.ToLower().TrimStart('.')
        $CookieDomain.Contains($TargetDomain) -or $TargetDomain.Contains($CookieDomain)
    }
    
    if (-not $FilteredCookies) {
        Write-Host "No cookies found matching domain '$Domain'." -ForegroundColor Yellow
        Write-Host "Available cookies in this tab belonged to domains: $(($Cookies.domain | Select-Object -Unique) -join ', ')" -ForegroundColor Yellow
        return
    }
    
    # Print cookies in a beautiful format
    Write-Host "`n=== Cookies for $Domain ===" -ForegroundColor Green
    
    # Print formatted Cookie Header
    $CookieHeaderItems = @()
    foreach ($Cookie in $FilteredCookies) {
        $CookieHeaderItems += "$($Cookie.name)=$($Cookie.value)"
    }
    $CookieHeader = $CookieHeaderItems -join "; "
    
    Write-Host "`n[Cookie Header String]:" -ForegroundColor Cyan
    Write-Host $CookieHeader
    
    Write-Host "`n[JSON Details (First 5 shown)]:" -ForegroundColor Cyan
    $FilteredCookies | Select-Object -First 5 | ConvertTo-Json -Depth 3

    # Sync to quotify.toml if requested
    if ($Sync) {
        Write-Host "`nSyncing cookie to configuration at $ConfigPath..." -ForegroundColor Gray
        $Target = if ($ResolvedTarget) { $ResolvedTarget } else { Resolve-ProviderTarget -ProviderName "" -TargetDomain $Domain }
        $MatchedProvider = if ($Target) { $Target.Provider } else { "" }
        $MatchedField = if ($Target) { $Target.Field } else { "" }

        if ([string]::IsNullOrWhiteSpace($MatchedProvider) -or [string]::IsNullOrWhiteSpace($MatchedField)) {
            Write-Warning "Could not map domain '$Domain' to a known provider. Supported cookie-based providers: mimo, opencode, abacus, alibabatoken, t3chat, amp, cursor."
        } else {
            Set-TomlField -Path $ConfigPath -Section $MatchedProvider -Field $MatchedField -Value $CookieHeader
            Write-Host "Successfully synced cookie to $ConfigPath for provider '$MatchedProvider' field '$MatchedField'." -ForegroundColor Green
        }
    }
} finally {
    if ($WebSocket.State -eq [System.Net.WebSockets.WebSocketState]::Open) {
        $WebSocket.CloseAsync([System.Net.WebSockets.WebSocketCloseStatus]::NormalClosure, "", $CancellationToken.Token).Wait()
    }
}
