# -Sid: SID string to round-trip (e.g. one Hydra derived for a box). Defaults to the SID of
# the account running the script, read at runtime, so no machine SID is stored here.
param([string]$Sid)
$ErrorActionPreference = 'Stop'
Add-Type @"
using System;
using System.Runtime.InteropServices;
public class A {
  [DllImport("advapi32.dll", CharSet=CharSet.Unicode, SetLastError=true)]
  public static extern bool GetUserNameW([MarshalAs(UnmanagedType.LPWStr)] System.Text.StringBuilder buf, ref uint size);
  [DllImport("advapi32.dll", CharSet=CharSet.Unicode, SetLastError=true)]
  public static extern bool LookupAccountNameW(string system, string account, IntPtr sid, ref uint sidSize,
      System.Text.StringBuilder domain, ref uint domainSize, out uint use);
  [DllImport("advapi32.dll", CharSet=CharSet.Unicode, SetLastError=true)]
  public static extern bool ConvertSidToStringSidW(IntPtr sid, out IntPtr text);
  [DllImport("advapi32.dll", CharSet=CharSet.Unicode, SetLastError=true)]
  public static extern bool ConvertStringSidToSidW(string text, out IntPtr sid);
  [DllImport("kernel32.dll", SetLastError=true)]
  public static extern IntPtr LocalFree(IntPtr mem);
  [DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true)]
  public static extern IntPtr GetModuleHandleW(string name);
  [DllImport("kernel32.dll", CharSet=CharSet.Ansi, SetLastError=true)]
  public static extern IntPtr GetProcAddress(IntPtr module, string name);
}
"@

$user = New-Object System.Text.StringBuilder 64
$size = [uint32]64
$ok = [A]::GetUserNameW($user, [ref]$size)
"GetUserNameW ok=$ok size=$size user='$($user.ToString())'"

$sidBuf = [System.Runtime.InteropServices.Marshal]::AllocHGlobal(128)
$sidSize = [uint32]128
$domain = New-Object System.Text.StringBuilder 64
$domainSize = [uint32]64
$use = [uint32]0
$ok2 = [A]::LookupAccountNameW($null, $user.ToString(), $sidBuf, [ref]$sidSize, $domain, [ref]$domainSize, [ref]$use)
"LookupAccountNameW ok=$ok2 sidSize=$sidSize domain='$($domain.ToString())' domainSize=$domainSize use=$use err=$([System.Runtime.InteropServices.Marshal]::GetLastWin32Error())"

$text = [IntPtr]::Zero
if ($ok2) {
  $ok3 = [A]::ConvertSidToStringSidW($sidBuf, [ref]$text)
  "ConvertSidToStringSidW ok=$ok3 text='$([System.Runtime.InteropServices.Marshal]::PtrToStringUni($text))'"
}

# Round-trip a SID string through ConvertStringSidToSidW: -Sid if given, else the SID read above.
$derived = if ($Sid) { $Sid } elseif ($text -ne [IntPtr]::Zero) { [System.Runtime.InteropServices.Marshal]::PtrToStringUni($text) } else { $null }
if (-not $derived) { throw "No SID to round-trip: LookupAccountNameW failed and -Sid was not given." }
$dsid = [IntPtr]::Zero
$ok4 = [A]::ConvertStringSidToSidW($derived, [ref]$dsid)
"ConvertStringSidToSidW ok=$ok4 err=$([System.Runtime.InteropServices.Marshal]::GetLastWin32Error())"
if ($ok4) {
  $dtext = [IntPtr]::Zero
  [void][A]::ConvertSidToStringSidW($dsid, [ref]$dtext)
  $bytes = [System.Runtime.InteropServices.Marshal]::ReadByte($dsid, 0)
  $count = [System.Runtime.InteropServices.Marshal]::ReadByte($dsid, 1)
  "derived sid size = 8 + 4*$count = $(8 + 4*$count) text='$([System.Runtime.InteropServices.Marshal]::PtrToStringUni($dtext))'"
  [void][A]::LocalFree($dtext)
  [void][A]::LocalFree($dsid)
}

# Exports: which modules carry each entry point.
$advapi = [A]::GetModuleHandleW('advapi32.dll')
$sechost = [A]::GetModuleHandleW('sechost.dll')
if ($sechost -eq [IntPtr]::Zero) { $sechost = [IntPtr]::Zero }
foreach ($name in 'GetUserNameW','GetUserNameA','ConvertSidToStringSidW','ConvertSidToStringSidA','ConvertStringSidToSidW','LookupAccountNameW','LookupAccountSidW') {
  $inAdvapi = [A]::GetProcAddress($advapi, $name) -ne [IntPtr]::Zero
  $inSechost = if ($sechost -ne [IntPtr]::Zero) { [A]::GetProcAddress($sechost, $name) -ne [IntPtr]::Zero } else { 'no-module' }
  "$name advapi32=$inAdvapi sechost=$inSechost"
}
if ($ok2) { [void][A]::LocalFree($sidBuf) }
if ($text -ne [IntPtr]::Zero) { [void][A]::LocalFree($text) }
