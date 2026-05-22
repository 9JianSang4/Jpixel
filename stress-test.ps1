<#
Jpixel 连续截图压测脚本

用之前：
  1. 先手工启动 Jpixel（cargo run 或者打开 exe）
  2. 确保热键是 F1（默认）
  3. 把焦点移到其他窗口（随便哪个），让 Jpixel 挂着
  4. 管理员运行这个脚本

注意：这脚本会疯狂触发 F1，运行的时候你基本没法用电脑。
#>

$hotkey = "F1"
$duration = 30     # 跑多少秒
$intervalMin = 50  # 最小间隔 ms — 模拟快速连按
$intervalMax = 300 # 最大间隔 ms

Write-Host "=== Jpixel 连续截图压测 ==="
Write-Host "热键: $hotkey"
Write-Host "持续时间: ${duration}s"
Write-Host "间隔范围: ${intervalMin}ms - ${intervalMax}ms"
Write-Host ""
Write-Host "5 秒后开始，赶紧切到别的窗口..."
Start-Sleep -Seconds 5

$shell = New-Object -ComObject WScript.Shell
$elapsed = [System.Diagnostics.Stopwatch]::StartNew()
$count = 0

while ($elapsed.Elapsed.TotalSeconds -lt $duration) {
    $shell.SendKeys("{$hotkey}")
    $count++
    $interval = Get-Random -Minimum $intervalMin -Maximum $intervalMax
    Start-Sleep -Milliseconds $interval
}

Write-Host ""
Write-Host "=== 完成 ==="
Write-Host "共发送 $count 次热键"
Write-Host "检查 Jpixel 是否还活着。如果卡死了，去看 jpixel.log"
