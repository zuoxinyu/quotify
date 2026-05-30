param (
    [Parameter(Mandatory=$true, HelpMessage="The domain to fetch cookies for (e.g. xiaomimimo.com)")]
    [string]$Domain,
    
    [int]$Port = 9222
)

$ErrorActionPreference = "Stop"

# Normalize the target domain for filtering
$TargetDomain = $Domain.ToLower().Trim().TrimStart('.')

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
} finally {
    if ($WebSocket.State -eq [System.Net.WebSockets.WebSocketState]::Open) {
        $WebSocket.CloseAsync([System.Net.WebSockets.WebSocketCloseStatus]::NormalClosure, "", $CancellationToken.Token).Wait()
    }
}
