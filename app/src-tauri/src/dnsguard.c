/*
 * Hydra DNS guard: loaded by Sandboxie (InjectDll64) into every process of a Hydra box.
 *
 * Name lookups in a box are handed to the Windows DNS client service outside the box (its
 * RPC endpoint is opened by Sandboxie's default template and cannot be closed by settings),
 * so a boxed process could resolve, and leak, any name on the host's DNS. This DLL refuses
 * connections to that endpoint. Windows then resolves inside the process, which sends its
 * queries with WSASendMsg to (v4-mapped) IPv6 addresses, a path Sandboxie's network rules do
 * not catch. So the DLL also refuses, in Winsock, every connect or send to an address that is
 * not loopback, which matches the box rules (only Hydra's local relay is allowed): external
 * names fail, while localhost, IP literals and the hosts file keep working. Claude itself
 * sends host names to its proxy and is unaffected.
 */
#include <winsock2.h>
#include <ws2tcpip.h>
#include <mswsock.h>
#include <windows.h>
#include <winternl.h>

#define STATUS_OBJECT_NAME_NOT_FOUND ((NTSTATUS)0xC0000034L)

typedef void *(*P_SbieDll_Hook)(const char *name, void *source, void *detour, HMODULE module);

typedef NTSTATUS(NTAPI *P_NtAlpcConnectPort)(PHANDLE, PUNICODE_STRING, POBJECT_ATTRIBUTES, PVOID, ULONG, PSID,
                                             PVOID, PSIZE_T, PVOID, PVOID, PLARGE_INTEGER);
typedef NTSTATUS(NTAPI *P_NtAlpcConnectPortEx)(PHANDLE, POBJECT_ATTRIBUTES, POBJECT_ATTRIBUTES, PVOID, ULONG,
                                               PSECURITY_DESCRIPTOR, PVOID, PSIZE_T, PVOID, PVOID, PLARGE_INTEGER);
typedef NTSTATUS(NTAPI *P_NtConnectPort)(PHANDLE, PUNICODE_STRING, PSECURITY_QUALITY_OF_SERVICE, PVOID, PVOID,
                                         PULONG, PVOID, PULONG);
typedef int(WSAAPI *P_connect)(SOCKET, const struct sockaddr *, int);
typedef int(WSAAPI *P_WSAConnect)(SOCKET, const struct sockaddr *, int, LPWSABUF, LPWSABUF, LPQOS, LPQOS);
typedef int(WSAAPI *P_sendto)(SOCKET, const char *, int, int, const struct sockaddr *, int);
typedef int(WSAAPI *P_WSASendTo)(SOCKET, LPWSABUF, DWORD, LPDWORD, DWORD, const struct sockaddr *, int,
                                 LPWSAOVERLAPPED, LPWSAOVERLAPPED_COMPLETION_ROUTINE);
typedef int(WSAAPI *P_WSASendMsg)(SOCKET, LPWSAMSG, DWORD, LPDWORD, LPWSAOVERLAPPED,
                                  LPWSAOVERLAPPED_COMPLETION_ROUTINE);
typedef void(WSAAPI *P_WSASetLastError)(int);

static P_NtAlpcConnectPort sys_NtAlpcConnectPort;
static P_NtAlpcConnectPortEx sys_NtAlpcConnectPortEx;
static P_NtConnectPort sys_NtConnectPort;
static P_connect sys_connect;
static P_WSAConnect sys_WSAConnect;
static P_sendto sys_sendto;
static P_WSASendTo sys_WSASendTo;
static P_WSASendMsg sys_WSASendMsg;
static P_WSASetLastError ws_set_error;

/* Whether `name` is the DNS client endpoint (`...\DNSResolver`, any case). */
static BOOL is_dns_port(const UNICODE_STRING *name) {
    static const WCHAR tail[] = L"\\dnsresolver";
    const USHORT n = (USHORT)(sizeof(tail) / sizeof(WCHAR) - 1);
    USHORT len, i;
    if (!name || !name->Buffer) return FALSE;
    len = name->Length / sizeof(WCHAR);
    if (len < n) return FALSE;
    for (i = 0; i < n; i++) {
        WCHAR c = name->Buffer[len - n + i];
        if (c >= L'A' && c <= L'Z') c += L'a' - L'A';
        if (c != tail[i]) return FALSE;
    }
    return TRUE;
}

static NTSTATUS NTAPI my_NtAlpcConnectPort(PHANDLE h, PUNICODE_STRING name, POBJECT_ATTRIBUTES oa, PVOID attrs,
                                           ULONG flags, PSID sid, PVOID msg, PSIZE_T len, PVOID out, PVOID in,
                                           PLARGE_INTEGER timeout) {
    if (is_dns_port(name)) return STATUS_OBJECT_NAME_NOT_FOUND;
    return sys_NtAlpcConnectPort(h, name, oa, attrs, flags, sid, msg, len, out, in, timeout);
}

static NTSTATUS NTAPI my_NtAlpcConnectPortEx(PHANDLE h, POBJECT_ATTRIBUTES port, POBJECT_ATTRIBUTES client,
                                             PVOID attrs, ULONG flags, PSECURITY_DESCRIPTOR sd, PVOID msg,
                                             PSIZE_T len, PVOID out, PVOID in, PLARGE_INTEGER timeout) {
    if (port && is_dns_port(port->ObjectName)) return STATUS_OBJECT_NAME_NOT_FOUND;
    return sys_NtAlpcConnectPortEx(h, port, client, attrs, flags, sd, msg, len, out, in, timeout);
}

static NTSTATUS NTAPI my_NtConnectPort(PHANDLE h, PUNICODE_STRING name, PSECURITY_QUALITY_OF_SERVICE qos,
                                       PVOID client, PVOID server, PULONG max, PVOID info, PULONG info_len) {
    if (is_dns_port(name)) return STATUS_OBJECT_NAME_NOT_FOUND;
    return sys_NtConnectPort(h, name, qos, client, server, max, info, info_len);
}

/*
 * Whether a destination is refused: any IPv4 or IPv6 address outside loopback, including
 * v4-mapped IPv6 (::ffff:a.b.c.d). A missing address (connected socket) or another family
 * (Unix sockets, Hyper-V) is left to Sandboxie.
 */
static BOOL refused(const struct sockaddr *a, int len) {
    if (!a || len < (int)sizeof(a->sa_family)) return FALSE;
    if (a->sa_family == AF_INET) {
        if (len < (int)sizeof(struct sockaddr_in)) return FALSE;
        return ((const unsigned char *)&((const struct sockaddr_in *)a)->sin_addr)[0] != 127;
    }
    if (a->sa_family == AF_INET6) {
        const unsigned char *b;
        int i;
        if (len < (int)sizeof(struct sockaddr_in6)) return FALSE;
        b = ((const struct sockaddr_in6 *)a)->sin6_addr.s6_addr;
        for (i = 0; i < 10 && b[i] == 0; i++) {}
        if (i == 10) {
            if (b[10] == 0xff && b[11] == 0xff) return b[12] != 127; /* v4-mapped */
            if (b[10] == 0 && b[11] == 0 && b[12] == 0 && b[13] == 0 && b[14] == 0 && b[15] == 1) return FALSE; /* ::1 */
        }
        return TRUE;
    }
    return FALSE;
}

static int refuse(int code) {
    if (ws_set_error) ws_set_error(code);
    return SOCKET_ERROR;
}

static int WSAAPI my_connect(SOCKET s, const struct sockaddr *a, int l) {
    if (refused(a, l)) return refuse(WSAECONNREFUSED);
    return sys_connect(s, a, l);
}

static int WSAAPI my_WSAConnect(SOCKET s, const struct sockaddr *a, int l, LPWSABUF cd, LPWSABUF ce, LPQOS q,
                                LPQOS g) {
    if (refused(a, l)) return refuse(WSAECONNREFUSED);
    return sys_WSAConnect(s, a, l, cd, ce, q, g);
}

static int WSAAPI my_sendto(SOCKET s, const char *buf, int len, int flags, const struct sockaddr *to, int tl) {
    if (refused(to, tl)) return refuse(WSAENETUNREACH);
    return sys_sendto(s, buf, len, flags, to, tl);
}

static int WSAAPI my_WSASendTo(SOCKET s, LPWSABUF bufs, DWORD n, LPDWORD sent, DWORD flags,
                               const struct sockaddr *to, int tl, LPWSAOVERLAPPED ov,
                               LPWSAOVERLAPPED_COMPLETION_ROUTINE cr) {
    if (refused(to, tl)) return refuse(WSAENETUNREACH);
    return sys_WSASendTo(s, bufs, n, sent, flags, to, tl, ov, cr);
}

static int WSAAPI my_WSASendMsg(SOCKET s, LPWSAMSG msg, DWORD flags, LPDWORD sent, LPWSAOVERLAPPED ov,
                                LPWSAOVERLAPPED_COMPLETION_ROUTINE cr) {
    if (msg && refused(msg->name, msg->namelen)) return refuse(WSAENETUNREACH);
    return sys_WSASendMsg(s, msg, flags, sent, ov, cr);
}

static void hook(P_SbieDll_Hook sbie_hook, HMODULE module, HMODULE self, const char *name, void *detour, void **sys) {
    void *source = module ? (void *)GetProcAddress(module, name) : NULL;
    if (source) *sys = sbie_hook(name, source, detour, self);
}

/* Called by Sandboxie after loading this DLL, with SbieDll's module handle. */
__declspec(dllexport) void __stdcall InjectDllMain(HINSTANCE sbie, ULONG_PTR unused) {
    HMODULE ntdll = GetModuleHandleW(L"ntdll.dll");
    HMODULE ws2 = LoadLibraryW(L"ws2_32.dll");
    HMODULE self = NULL;
    P_SbieDll_Hook sbie_hook = (P_SbieDll_Hook)GetProcAddress((HMODULE)sbie, "SbieDll_Hook");
    (void)unused;
    if (!sbie_hook || !ntdll) return;
    GetModuleHandleExW(GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
                       (LPCWSTR)(void *)&InjectDllMain, &self);
    hook(sbie_hook, ntdll, self, "NtAlpcConnectPort", (void *)my_NtAlpcConnectPort, (void **)&sys_NtAlpcConnectPort);
    hook(sbie_hook, ntdll, self, "NtAlpcConnectPortEx", (void *)my_NtAlpcConnectPortEx, (void **)&sys_NtAlpcConnectPortEx);
    hook(sbie_hook, ntdll, self, "NtConnectPort", (void *)my_NtConnectPort, (void **)&sys_NtConnectPort);
    if (!ws2) return;
    ws_set_error = (P_WSASetLastError)GetProcAddress(ws2, "WSASetLastError");
    hook(sbie_hook, ws2, self, "connect", (void *)my_connect, (void **)&sys_connect);
    hook(sbie_hook, ws2, self, "WSAConnect", (void *)my_WSAConnect, (void **)&sys_WSAConnect);
    hook(sbie_hook, ws2, self, "sendto", (void *)my_sendto, (void **)&sys_sendto);
    hook(sbie_hook, ws2, self, "WSASendTo", (void *)my_WSASendTo, (void **)&sys_WSASendTo);
    hook(sbie_hook, ws2, self, "WSASendMsg", (void *)my_WSASendMsg, (void **)&sys_WSASendMsg);
}

BOOL WINAPI DllMain(HINSTANCE inst, DWORD reason, LPVOID reserved) {
    (void)reserved;
    if (reason == DLL_PROCESS_ATTACH) DisableThreadLibraryCalls(inst);
    return TRUE;
}
