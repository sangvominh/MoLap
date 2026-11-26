$ErrorActionPreference = 'SilentlyContinue'

function Format-Rate([double]$bytesPerSec) {
    if ($bytesPerSec -ge 1048576) {
        return ('{0:N1} MB/s' -f ($bytesPerSec / 1048576))
    }
    if ($bytesPerSec -ge 1024) {
        return ('{0:N0} KB/s' -f ($bytesPerSec / 1024))
    }
    return ('{0:N0} B/s' -f $bytesPerSec)
}

$ifaces = [System.Net.NetworkInformation.NetworkInterface]::GetAllNetworkInterfaces() | Where-Object {
    $_.OperationalStatus -eq 'Up' -and
    $_.NetworkInterfaceType -ne 'Loopback' -and
    $_.NetworkInterfaceType -ne 'Tunnel'
}

if (-not $ifaces) {
    Write-Output 'title=Network Pulse'
    Write-Output 'badge=OFFLINE'
    Write-Output 'tone=warn'
    Write-Output 'value=Disconnected'
    Write-Output 'subtitle=No active network adapter detected'
    Write-Output 'alert=false'
    exit 0
}

$rx1 = 0L; $tx1 = 0L
foreach ($n in $ifaces) {
    $s = $n.GetIPv4Statistics()
    $rx1 += $s.BytesReceived
    $tx1 += $s.BytesSent
}

Start-Sleep -Milliseconds 200

$rx2 = 0L; $tx2 = 0L
foreach ($n in $ifaces) {
    $s = $n.GetIPv4Statistics()
    $rx2 += $s.BytesReceived
    $tx2 += $s.BytesSent
}

$rxRate = [Math]::Max(0.0, ($rx2 - $rx1) / 0.2)
$txRate = [Math]::Max(0.0, ($tx2 - $tx1) / 0.2)
$primary = $ifaces | Sort-Object Speed -Descending | Select-Object -First 1
$linkMbps = [Math]::Round($primary.Speed / 1000000)

Write-Output 'title=Network Pulse'
Write-Output 'badge=ONLINE'
Write-Output 'tone=good'
Write-Output ('value={0} down' -f (Format-Rate $rxRate))
Write-Output ('subtitle={0}' -f $primary.Name)
Write-Output ('pill=NET {0}' -f (Format-Rate $rxRate))
Write-Output 'alert=false'
Write-Output ('item=Download | {0}' -f (Format-Rate $rxRate))
Write-Output ('item=Upload | {0}' -f (Format-Rate $txRate))
Write-Output ('item=Link Speed | {0} Mbps' -f $linkMbps)
