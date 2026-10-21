param([int]$Seconds = 20)

Add-Type @"
using System;
using System.Runtime.InteropServices;
public static class KeyProbe {
  [DllImport("user32.dll")]
  public static extern short GetAsyncKeyState(int vKey);
}
"@

$names = @{
  0x08='Backspace'; 0x09='Tab'; 0x0D='Enter'; 0x10='Shift'; 0x11='Ctrl'; 0x12='Alt'
  0x13='Pause'; 0x1B='Esc'; 0x20='Space'; 0x21='PageUp'; 0x22='PageDown'; 0x23='End'
  0x24='Home'; 0x25='Left'; 0x26='Up'; 0x27='Right'; 0x28='Down'; 0x2C='PrintScreen'
  0x2D='Insert'; 0x2E='Delete'; 0x2F='HELP'
  0x30='0'; 0x31='1'; 0x32='2'; 0x33='3'; 0x34='4'; 0x35='5'; 0x36='6'; 0x37='7'
  0x38='8'; 0x39='9'
  0x41='A'; 0x42='B'; 0x43='C'; 0x44='D'; 0x45='E'; 0x46='F'; 0x47='G'; 0x48='H'
  0x49='I'; 0x4A='J'; 0x4B='K'; 0x4C='L'; 0x4D='M'; 0x4E='N'; 0x4F='O'; 0x50='P'
  0x51='Q'; 0x52='R'; 0x53='S'; 0x54='T'; 0x55='U'; 0x56='V'; 0x57='W'; 0x58='X'
  0x59='Y'; 0x5A='Z'
  0x60='Num0'; 0x61='Num1'; 0x62='Num2'; 0x63='Num3'; 0x64='Num4'; 0x65='Num5'
  0x66='Num6'; 0x67='Num7'; 0x68='Num8'; 0x69='Num9'; 0x6A='Num*'; 0x6B='Num+'
  0x6D='Num-'; 0x6E='Num.'; 0x6F='Num/'
  0x70='F1'; 0x71='F2'; 0x72='F3'; 0x73='F4'; 0x74='F5'; 0x75='F6'
  0x76='F7'; 0x77='F8'; 0x78='F9'; 0x79='F10'; 0x7A='F11'; 0x7B='F12'
  0x90='NumLock'; 0x91='ScrollLock'
  0xA0='LShift'; 0xA1='RShift'; 0xA2='LCtrl'; 0xA3='RCtrl'; 0xA4='LAlt'; 0xA5='RAlt'
  0xA6='BrowserBack'; 0xA7='BrowserForward'; 0xA8='BrowserRefresh'; 0xA9='BrowserStop'
  0xAA='BrowserSearch'; 0xAB='BrowserFavorites'; 0xAC='BrowserHome'
  0xAD='VolumeMute'; 0xAE='VolumeDown'; 0xAF='VolumeUp'
  0xB0='MediaNext'; 0xB1='MediaPrev'; 0xB2='MediaStop'; 0xB3='MediaPlayPause'
  0xB4='LaunchMail'; 0xB5='LaunchMedia'; 0xB6='LaunchApp1'; 0xB7='LaunchApp2'
  0xBA=';'; 0xBB='='; 0xBC=','; 0xBD='-'; 0xBE='.'; 0xBF='/'
  0xC0='`'; 0xDB='['; 0xDC='\\'; 0xDD=']'; 0xDE="'"
}

Write-Host ""
Write-Host "Jpixel key probe: press F1, F8, Ctrl+Alt+J, and Fn+F1 now."
Write-Host "Logging for $Seconds seconds. Only key-down transitions are printed."
Write-Host ""

$start = Get-Date
$prev = @{}
while (((Get-Date) - $start).TotalSeconds -lt $Seconds) {
    $elapsed = ((Get-Date) - $start).TotalSeconds
    for ($vk = 0x08; $vk -le 0xFE; $vk++) {
        $down = ([KeyProbe]::GetAsyncKeyState($vk) -band 0x8000) -ne 0
        if ($down -and -not $prev[$vk]) {
            $name = if ($names.ContainsKey($vk)) { $names[$vk] } else { '' }
            Write-Host ("{0,6:N2}s  VK=0x{1:X2}  {2}" -f $elapsed, $vk, $name)
        }
        $prev[$vk] = $down
    }
    Start-Sleep -Milliseconds 15
}

Write-Host ""
Write-Host "done"
