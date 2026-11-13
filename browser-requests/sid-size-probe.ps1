$ErrorActionPreference = 'Stop'
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
using System.Text;

public class S {
    [DllImport("advapi32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    public static extern bool GetUserNameW(StringBuilder buf, ref uint size);

    [DllImport("advapi32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    public static extern bool LookupAccountNameW(string system, string account, IntPtr sid, ref uint sidSize,
        StringBuilder domain, ref uint domainSize, out int use);
}

public static class Probe {
    public static void Run() {
        var buf = new StringBuilder(64);
        uint size = 64;
        bool ok = S.GetUserNameW(buf, ref size);
        Console.WriteLine("GetUserNameW ok=" + ok + " size=" + size + " name=" + buf.ToString());

        string account = buf.ToString();
        foreach (uint cap in new uint[] { 28, 40, 68, 128 }) {
            IntPtr sid = Marshal.AllocHGlobal(256);
            try {
                for (int i = 0; i < 256; i++) Marshal.WriteByte(sid, i, 0);
                uint sidSize = cap;
                var domain = new StringBuilder(64);
                uint domainSize = 64;
                int use;
                bool r = S.LookupAccountNameW(null, account, sid, ref sidSize, domain, ref domainSize, out use);
                byte rev = Marshal.ReadByte(sid, 0);
                byte count = Marshal.ReadByte(sid, 1);
                Console.WriteLine("cap=" + cap + " ok=" + r + " sidSize=" + sidSize
                    + " bytes(b1=count)=" + count + " computed=" + (8 + 4 * count)
                    + " domainSize=" + domainSize + " domain=" + domain.ToString() + " use=" + use
                    + " err=" + Marshal.GetLastWin32Error());
            } finally { Marshal.FreeHGlobal(sid); }
        }
    }
}
'@
[Probe]::Run()
